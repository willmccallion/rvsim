"""Run a workload in phases: fast-forward to a point of interest, switch to
the configuration under study, warm it up, and measure regions.

A :class:`Session` owns the running simulator and keeps its console, so a
script can type into a guest shell and wait for output. Fast-forwards are
cached: the checkpoint at the stop is saved under a key made of the
workload's files, everything the session did before, the fast-forward
configuration and the stop, so the next session with the same history
restores it in seconds instead of running again."""

from __future__ import annotations

import copy
import datetime
import json
import os
import pathlib
import re
import sys
import tempfile
import time
from dataclasses import dataclass
from typing import Any, Callable, Dict, Iterator, List, Mapping, Optional, TextIO, Tuple, Union

from .. import _core, presets
from ..config import Config
from ..config._config import _config_to_dict
from ..simulator import Simulator
from .cache import _META_FORMAT, _Cache, _read_meta, default_cache_dir
from .stops import (
    LOGIN_SHELL,
    Console,
    Cycles,
    Exit,
    Instructions,
    Marker,
    Pc,
    Stop,
    Stopped,
    When,
)
from .workload import Region, Workload, _digest_json


ConfigLike = Union[Config, Dict[str, Any]]


#: Native config fields the guest can observe; a switch keeps them.
_GUEST_VISIBLE = (
    ("system", "hart_count"),
    ("system", "ram_base"),
    ("system", "uart_base"),
    ("system", "disk_base"),
    ("system", "clint_base"),
    ("system", "syscon_base"),
    ("system", "sim_control_base"),
    ("memory", "ram_size"),
    ("memory", "misaligned_access_trap"),
    ("memory", "paging_mode_max"),
    ("pipeline", "vlen"),
    ("isa", "svadu"),
)


#: Fields a Linux session's fast-forward system takes from its own config.
_LINUX_FAST_FORWARD_KEEPS = (
    ("memory", "ram_size"),
    ("memory", "misaligned_access_trap"),
    ("memory", "paging_mode_max"),
    ("pipeline", "vlen"),
    ("isa", "svadu"),
)


#: The first stats-dump label a session uses, clear of labels guests pick.
_FIRST_LABEL = 1 << 40


_PROGRESS_SECONDS = 10.0


_PROGRESS_CYCLES = 5_000_000


class WorkloadEnded(RuntimeError):
    """The workload ended before the run reached its stop."""

    def __init__(self, stop: Stop, stopped: Stopped):
        super().__init__(
            f"the workload ended with exit code {stopped.exit_code} before reaching {stop}"
        )
        self.stop = stop
        self.stopped = stopped


def _native(config: ConfigLike) -> Dict[str, Any]:
    """The native config dict, with the console captured for the session."""
    native = copy.deepcopy(_config_to_dict(config))
    native["system"]["console"] = "captured"
    return native


_ECHOED_COMMAND_END = re.compile(r"\$\?\r?\n")


def _typed_command_output(console: str) -> str:
    """What a command typed as ``...; echo TOKEN $?`` printed, given the
    console from where it was typed: everything after the terminal's echo
    of the line, which it may have wrapped."""
    echo_end = _ECHOED_COMMAND_END.search(console)
    return console[echo_end.end() :] if echo_end else ""


def _guest_view_differences(a: Dict[str, Any], b: Dict[str, Any]) -> List[str]:
    return [
        f"{field}: {a[section][field]!r} vs {b[section][field]!r}"
        for section, field in _GUEST_VISIBLE
        if a[section][field] != b[section][field]
    ]


def _repo_linux_dir() -> Optional[pathlib.Path]:
    here = pathlib.Path(__file__).resolve().parent.parent.parent
    for root in (here, pathlib.Path.cwd()):
        candidate = root / "software" / "linux" / "output"
        if candidate.is_dir():
            return candidate
    return None


@dataclass(frozen=True)
class ShellResult:
    """A shell command's output and exit status."""

    output: str
    exit_code: int


@dataclass(frozen=True)
class FastForward:
    """How a fast-forward reached its stop."""

    stop: Stop
    cached: bool
    """Whether the stop was restored from the cache instead of run to."""
    checkpoint: Optional[str]
    """The cached checkpoint, when the fast-forward was cached."""
    cycle: int
    instructions: int
    host_seconds: float


class Session:
    """A workload run in phases on one or more configurations.

    Example::

        s = Session.linux(harts=8)
        s.fast_forward(until=Session.LOGIN_SHELL)      # cached after the first time
        s.switch(presets.linux(harts=8, core=my_core))
        s.warm_up(command="coremark 0x0 0x0 0x66 20 7 1 2000")
        r = s.measure("coremark 0x0 0x0 0x66 20 7 1 2000")
        print(r.ipc, r.stats["core0.bp.committed.accuracy"], r.exit_code)

    The session runs on ``config``. Fast-forwards run on
    ``fast_forward_config`` (``config`` unless given) and return to
    ``config`` at their stop, so what a session measures always runs on the
    configuration it was given or last switched to. Direct changes made
    through :attr:`sim` are not part of the history cache keys are made
    of; fast-forward with ``cache=False`` after them.
    """

    LOGIN_SHELL = LOGIN_SHELL

    def __init__(
        self,
        config: Optional[ConfigLike] = None,
        *,
        binary: Optional[str] = None,
        kernel: Optional[str] = None,
        firmware: Optional[str] = None,
        disk: Optional[str] = None,
        dtb: Optional[str] = None,
        fast_forward_config: Optional[ConfigLike] = None,
        cache_dir: Optional[str] = None,
        echo: Union[bool, TextIO] = False,
        console_log: Optional[str] = None,
        progress: bool = False,
    ):
        """
        Args:
            config: The configuration to run and measure on (``Config()``
                by default).
            binary: A bare-metal ELF to run.
            kernel: A kernel image to boot, through ``firmware`` (OpenSBI
                ``fw_jump``; ``fw_jump.bin`` beside the kernel by default),
                with an optional ``disk`` image and ``dtb``.
            fast_forward_config: The configuration fast-forwards run on.
            cache_dir: Where cached fast-forward checkpoints live
                (:func:`default_cache_dir` by default).
            echo: Copy the console to stdout (``True``) or to a stream.
            console_log: Write the console to this file.
            progress: Report long runs' progress on stderr.
        """
        workload = Workload(binary=binary, kernel=kernel, firmware=firmware, disk=disk, dtb=dtb)
        self._init(workload, config, fast_forward_config, cache_dir, echo, console_log, progress)

    def _init(
        self,
        workload: Workload,
        config: Optional[ConfigLike],
        fast_forward_config: Optional[ConfigLike],
        cache_dir: Optional[str],
        echo: Union[bool, TextIO],
        console_log: Optional[str],
        progress: bool,
    ) -> None:
        self.workload = workload
        self._config_given: ConfigLike = config if config is not None else Config()
        self._config = _native(self._config_given)
        self._fast_forward_config = (
            _native(fast_forward_config) if fast_forward_config is not None else self._config
        )
        differences = _guest_view_differences(self._config, self._fast_forward_config)
        if differences:
            raise ValueError(
                "the fast-forward config must show the guest the same system: "
                + "; ".join(differences)
            )
        self._cache = _Cache(cache_dir or default_cache_dir())
        self._echo: Optional[TextIO] = sys.stdout if echo is True else (echo or None)
        self._log = open(console_log, "w") if console_log else None
        self._progress = progress
        self._sim: Optional[Simulator] = None
        self._sim_config: Optional[Dict[str, Any]] = None
        self._console = ""
        self._cursor = 0
        self._history: List[Dict[str, Any]] = []
        self._untracked: Optional[str] = None
        self._next_label = _FIRST_LABEL
        self._phase: Optional[str] = None
        self.last_fast_forward: Optional[FastForward] = None

    @classmethod
    def linux(
        cls,
        config: Optional[ConfigLike] = None,
        *,
        harts: Optional[int] = None,
        image_dir: Optional[str] = None,
        kernel: Optional[str] = None,
        firmware: Optional[str] = None,
        disk: Optional[str] = None,
        **kwargs: Any,
    ) -> "Session":
        """A session on the bundled Linux image (built by ``make linux``).

        ``config`` defaults to ``presets.linux(harts)``. Fast-forwards run
        on ``presets.linux`` with its timer ticking every cycle, which
        compresses the boot's sleeps, and with ``config``'s RAM, VLEN and
        ISA options, so the cached boot is shared by every core
        configuration of the same system.
        """
        if config is None:
            config = presets.linux(harts if harts is not None else 8)
        elif harts is not None:
            raise ValueError("pass harts or a config, not both")
        native = _native(config)
        directory = pathlib.Path(image_dir) if image_dir else _repo_linux_dir()
        if directory is None and (kernel is None or disk is None):
            raise FileNotFoundError("no software/linux/output found; build the image with `make linux`")

        def default(path: Optional[str], name: str) -> Optional[str]:
            if path is not None:
                return path
            return str(directory / name) if directory is not None else None

        if "fast_forward_config" not in kwargs:
            fast = _native(presets.linux(native["system"]["hart_count"], real_time=False))
            for section, field in _LINUX_FAST_FORWARD_KEEPS:
                fast[section][field] = native[section][field]
            kwargs["fast_forward_config"] = fast
        return cls(
            config,
            kernel=default(kernel, "Image"),
            firmware=default(firmware, "fw_jump.bin"),
            disk=default(disk, "disk.img"),
            **kwargs,
        )

    @classmethod
    def resume(cls, path: str, config: Optional[ConfigLike] = None, **kwargs: Any) -> "Session":
        """A session continuing from a checkpoint :meth:`save` wrote, on
        ``config`` (the configuration it was saved on by default).

        Raises ``ValueError`` if the workload's files changed since."""
        meta = _read_meta(path)
        workload = Workload(**meta["workload"]["files"])
        if workload.digests() != meta["workload"]["digests"]:
            raise ValueError(f"the workload's files changed since {path} was saved")
        session = cls.__new__(cls)
        session._init(
            workload,
            config if config is not None else meta["config"],
            kwargs.pop("fast_forward_config", None),
            kwargs.pop("cache_dir", None),
            kwargs.pop("echo", False),
            kwargs.pop("console_log", None),
            kwargs.pop("progress", False),
        )
        if kwargs:
            raise TypeError(f"unexpected arguments: {sorted(kwargs)}")
        session._load(path, meta)
        return session

    @property
    def sim(self) -> Simulator:
        """The running simulator, built on first use."""
        if self._sim is None:
            self._run_on(self._config)
        assert self._sim is not None
        return self._sim

    @property
    def config(self) -> ConfigLike:
        """The configuration the session runs and measures on."""
        return self._config_given

    @property
    def console(self) -> str:
        """Everything the guest has printed."""
        return self._console

    @property
    def stats(self) -> Any:
        """The running simulator's stats since it was built or restored."""
        return self.sim.stats

    @property
    def cycle(self) -> int:
        return self.sim.cycle

    @property
    def instructions(self) -> int:
        return self.sim.instructions_retired

    def run(
        self,
        until: Optional[Stop] = None,
        *,
        every: Optional[int] = None,
        on_every: Optional[Callable[["Session"], None]] = None,
    ) -> Stopped:
        """Runs until ``until`` holds (the workload's end by default),
        calling ``on_every(session)`` every ``every`` cycles on the way."""
        if (every is None) != (on_every is None):
            raise ValueError("every and on_every go together")
        stop = until if until is not None else Exit()
        if every is not None:
            return self._run_primitives(stop._primitives(), every=every, on_every=on_every)
        return stop._drive(self)

    def fast_forward(self, until: Stop, *, cache: bool = True) -> "Session":
        """Gets to ``until`` quickly: restores it from the cache when this
        history has reached it before, else runs to it on the fast-forward
        configuration (and caches it). Either way the session continues
        from the stop's checkpoint on its own configuration, with caches,
        TLBs and predictors cold, so what follows does not depend on
        whether the cache held the stop.

        Raises :class:`WorkloadEnded` if the workload ends first."""
        began = time.perf_counter()
        key = self._cache_key(until) if cache else None
        if key is not None and self._cache.has(key):
            path = self._cache.path(key)
            self._load(path, _read_meta(path))
            self._record_fast_forward(until, True, path, began)
            return self
        self._run_on(self._fast_forward_config)
        stopped = self._drive_logged(until, "fast-forward")
        if stopped.exited and not isinstance(until, Exit):
            raise WorkloadEnded(until, stopped)
        if key is not None:
            path = self._cache.path(key)
            self._save(path)
            self._load(path, _read_meta(path))
        else:
            path = None
            with tempfile.TemporaryDirectory(prefix="rvsim-fast-forward-") as directory:
                stop_point = os.path.join(directory, "stop.ckpt")
                self._save(stop_point)
                self._load(stop_point, _read_meta(stop_point))
        self._record_fast_forward(until, False, path, began)
        return self

    def switch(self, config: ConfigLike) -> "Session":
        """Continues on ``config``, which must show the guest the same
        system (harts, RAM, memory map, VLEN and ISA options). Caches,
        TLBs and predictors start cold."""
        native = _native(config)
        differences = _guest_view_differences(self._sim_config or self._config, native)
        if differences:
            raise ValueError(
                "a switch must keep the system the guest sees; "
                + "; ".join(differences)
                + (". For Linux, wrap a core config with presets.linux(core=...)" if self.workload.is_kernel else "")
            )
        self._config_given, self._config = config, native
        if self._sim is not None:
            self._run_on(native)
        return self

    def warm_up(self, until: Optional[Stop] = None, *, command: Optional[str] = None) -> "Session":
        """Runs unmeasured to ``until``, or through a shell ``command``, to
        warm caches and predictors before measuring."""
        if (until is None) == (command is None):
            raise ValueError("warm_up takes an until stop or a command, exactly one")
        if command is not None:
            self.shell(command)
        else:
            stopped = self._drive_logged(until, "warm-up")
            if stopped.exited and not isinstance(until, Exit):
                raise WorkloadEnded(until, stopped)
        return self

    def measure(
        self,
        command: Optional[str] = None,
        *,
        until: Optional[Stop] = None,
        name: Optional[str] = None,
    ) -> Region:
        """Measures a shell ``command`` (Linux) or the run to ``until``.

        A command runs under the guest's ``rvsim run``, which snapshots the
        stats just before it starts and just after it exits; the region is
        the difference, so the session's whole-run stats stay intact."""
        if (command is None) == (until is None):
            raise ValueError("measure takes a command or an until stop, exactly one")
        began = time.perf_counter()
        if command is not None:
            return self._measure_command(command, name or command, began)
        return self._measure_run(until, name or str(until), began)

    def send(self, text: str) -> None:
        """Types ``text`` into the console."""
        self.sim.write_console(text)
        self._history.append({"send": text})

    def expect(self, pattern: Union[str, "re.Pattern[str]"]) -> "re.Match[str]":
        """Runs until the console prints ``pattern`` and returns the match.

        Raises :class:`WorkloadEnded` if the workload ends first."""
        stop = Console(pattern)
        stopped = self.run(stop)
        if stopped.exited:
            raise WorkloadEnded(stop, stopped)
        assert stopped.match is not None
        return stopped.match

    def shell(self, command: str) -> ShellResult:
        """Runs ``command`` in the guest's shell and waits for it."""
        self._require_shell()
        token = f"__rvsim_{self._take_labels()[0]}__"
        start = len(self._console)
        self.send(f"{command}; echo {token} $?\n")
        match = self.expect(rf"{re.escape(token)} (\d+)\r?\n")
        return ShellResult(self._command_output(start, match), int(match.group(1)))

    def save(self, path: str) -> str:
        """Saves the session's state to ``path`` (with its console and
        history beside it in ``path.json``) for :meth:`resume`.

        Saving drains the pipelines first, as gem5 does, so this session
        continues a few cycles later than it would have; a session resumed
        from ``path`` starts with caches, TLBs and predictors cold."""
        self._save(path)
        return path

    def fork(self, configs: Mapping[str, ConfigLike]) -> Iterator[Tuple[str, "Session"]]:
        """Continues from this point once per configuration: yields
        ``(name, session)`` pairs, each an independent session on its
        config. This session is left as it was."""
        with tempfile.TemporaryDirectory(prefix="rvsim-fork-") as directory:
            path = os.path.join(directory, "fork.ckpt")
            self._save(path)
            meta = _read_meta(path)
            for name, config in configs.items():
                child = Session.__new__(Session)
                child._init(
                    self.workload,
                    config,
                    self._fast_forward_config,
                    self._cache.directory,
                    self._echo or False,
                    None,
                    self._progress,
                )
                child._load(path, meta)
                yield name, child

    def close(self) -> None:
        if self._log is not None:
            self._log.close()
            self._log = None

    def __enter__(self) -> "Session":
        return self

    def __exit__(self, *exc: Any) -> None:
        self.close()

    def _run_on(self, config: Dict[str, Any]) -> None:
        """Makes ``config`` the running configuration, carrying the state
        over from the current one."""
        if self._sim is None:
            self._sim = self.workload.build(config)
            self._sim_config = config
            self._history.append({"boot": _digest_json(config)})
            return
        if self._sim_config == config:
            return
        with tempfile.TemporaryDirectory(prefix="rvsim-switch-") as directory:
            path = os.path.join(directory, "switch.ckpt")
            self._sim.save(path)
            sim = self.workload.build(config)
            sim.restore(path)
        self._sim, self._sim_config = sim, config
        self._history.append({"config": _digest_json(config)})

    def _cache_key(self, until: Stop) -> str:
        stop_key = until.key()
        if stop_key is None:
            raise ValueError(f"{until} has no stable key to cache by; name it or pass cache=False")
        if self._untracked is not None:
            raise ValueError(
                f"this session's history includes {self._untracked}, which a cache key "
                "cannot capture; pass cache=False"
            )
        return _digest_json(
            {
                "checkpoint_version": _core.CHECKPOINT_VERSION,
                "workload": self.workload.digests(),
                "history": self._history,
                "fast_forward_config": _digest_json(self._fast_forward_config),
                "until": stop_key,
            }
        )

    def _record_fast_forward(
        self, until: Stop, cached: bool, path: Optional[str], began: float
    ) -> None:
        self.last_fast_forward = FastForward(
            stop=until,
            cached=cached,
            checkpoint=path,
            cycle=self.sim.cycle,
            instructions=self.sim.instructions_retired,
            host_seconds=time.perf_counter() - began,
        )
        if self._progress:
            source = "restored from the cache" if cached else "ran"
            self._say(
                f"fast-forward to {until}: {source} in {self.last_fast_forward.host_seconds:.1f}s, "
                f"cycle {self.sim.cycle:,}"
            )

    def _drive_logged(self, stop: Stop, phase: str) -> Stopped:
        self._phase = phase
        try:
            return stop._drive(self)
        finally:
            self._phase = None

    def _run_primitives(
        self,
        stops: Tuple[Stop, ...],
        *,
        every: Optional[int] = None,
        on_every: Optional[Callable[["Session"], None]] = None,
    ) -> Stopped:
        """Runs until one of ``stops`` holds; see :mod:`rvsim.stops`."""
        keys = [stop.key() for stop in stops]
        if None in keys or every is not None:
            self._untracked = self._untracked or f"a run to {stops}"
        self._history.append({"run": sorted(str(key) for key in keys)})
        try:
            return _Run(self, stops, every, on_every).until_stopped()
        except BaseException:
            self._untracked = self._untracked or f"an interrupted run to {stops}"
            raise

    def _take_console(self) -> None:
        text = self.sim.read_console()
        if not text:
            return
        self._console += text
        if self._echo is not None:
            self._echo.write(text)
            self._echo.flush()
        if self._log is not None:
            self._log.write(text)
            self._log.flush()

    def _match_console(self, consoles: List[Console]) -> Optional[Tuple[Console, "re.Match[str]"]]:
        best: Optional[Tuple[Console, "re.Match[str]"]] = None
        for console in consoles:
            match = console.regex.search(self._console, self._cursor)
            if match and (best is None or match.start() < best[1].start()):
                best = (console, match)
        if best is not None:
            self._cursor = best[1].end()
        return best

    def _require_shell(self) -> None:
        if not self.workload.is_kernel:
            raise ValueError("commands need a Linux shell; measure a bare-metal run with until=")

    def _take_labels(self) -> Tuple[int, int]:
        start = self._next_label
        self._next_label += 2
        return start, start + 1

    def _command_output(self, start: int, match: "re.Match[str]") -> str:
        return _typed_command_output(self._console[start : match.start()])

    def _measure_command(self, command: str, name: str, began: float) -> Region:
        self._require_shell()
        first, last = self._take_labels()
        token = f"__rvsim_{first}__"
        start = len(self._console)
        self.send(f"rvsim run {first} {last} {command}; echo {token} $?\n")
        match = self.expect(rf"{re.escape(token)} (\d+)\r?\n")
        output = self._command_output(start, match)
        try:
            stats = self.sim.stats_between(first, last)
        except KeyError:
            raise RuntimeError(
                f"the guest took no stats snapshots around {command!r}; "
                f"is the rvsim guest tool on the image? It printed: {output!r}"
            ) from None
        return Region(name, stats, output, int(match.group(1)), None, time.perf_counter() - began)

    def _measure_run(self, until: Stop, name: str, began: float) -> Region:
        before = self.sim.stats
        start = len(self._console)
        stopped = self._drive_logged(until, f"measure {name}")
        region = self.sim.stats - before
        output = self._console[start:]
        return Region(name, region, output, stopped.exit_code, stopped, time.perf_counter() - began)

    def _save(self, path: str) -> None:
        """Writes the checkpoint and its metadata, each atomically."""
        directory = os.path.dirname(os.path.abspath(path))
        os.makedirs(directory, exist_ok=True)
        pending = f"{path}.{os.getpid()}.partial"
        self.sim.save(pending)
        meta = {
            "format": _META_FORMAT,
            "checkpoint_version": _core.CHECKPOINT_VERSION,
            "rvsim_version": _core.version(),
            "created": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "workload": {"files": self.workload.files(), "digests": self.workload.digests()},
            "config": self._sim_config,
            "history": self._history,
            "untracked": self._untracked,
            "console": self._console,
            "cursor": self._cursor,
            "next_label": self._next_label,
            "cycle": self.sim.cycle,
            "instructions": self.sim.instructions_retired,
        }
        with open(pending + ".json", "w") as f:
            json.dump(meta, f)
        os.replace(pending, path)
        os.replace(pending + ".json", path + ".json")

    def _load(self, path: str, meta: Dict[str, Any]) -> None:
        """Replaces the running state with a saved one, continuing on the
        session's configuration."""
        differences = _guest_view_differences(meta["config"], self._config)
        if differences:
            raise ValueError(f"{path} was saved on a different system: " + "; ".join(differences))
        sim = self.workload.build(self._config)
        sim.restore(path)
        self._sim, self._sim_config = sim, self._config
        self._console = meta["console"]
        self._cursor = meta["cursor"]
        self._history = list(meta["history"])
        self._untracked = meta["untracked"]
        self._next_label = meta["next_label"]
        if _digest_json(meta["config"]) != _digest_json(self._config):
            self._history.append({"config": _digest_json(self._config)})
        if self._log is not None:
            self._log.write(self._console)
            self._log.flush()

    def _say(self, message: str) -> None:
        sys.stderr.write(f"[rvsim] {message}\n")
        sys.stderr.flush()


class _Run:
    """One run of a session until one of its primitive stops holds."""

    def __init__(
        self,
        session: Session,
        stops: Tuple[Stop, ...],
        every: Optional[int],
        on_every: Optional[Callable[[Session], None]],
    ):
        self.session = session
        self.sim = session.sim
        start_cycle = self.sim.cycle
        start_instructions = self.sim.instructions_retired
        self.cycle_targets = [(start_cycle + s.count, s) for s in stops if isinstance(s, Cycles)]
        self.instruction_targets = [
            (start_instructions + s.count, s) for s in stops if isinstance(s, Instructions)
        ]
        self.pc_stops = [s for s in stops if isinstance(s, Pc)]
        self.markers = [s for s in stops if isinstance(s, Marker)]
        self.consoles = [s for s in stops if isinstance(s, Console)]
        self.whens = [[start_cycle + s.every, s] for s in stops if isinstance(s, When)]
        self.exit = next((s for s in stops if isinstance(s, Exit)), Exit())
        self.every = every
        self.on_every = on_every
        self.next_every = start_cycle + every if every is not None else None
        self.started = time.perf_counter()
        self.start_cycle = start_cycle
        self.last_report = self.started

    def until_stopped(self) -> Stopped:
        while True:
            stopped = self._reached()
            if stopped is not None:
                return stopped
            reason, value = self.sim.run_to(
                cycles=self._cycles_to_run(),
                instructions=self._instructions_to_run(),
                pc=[a for stop in self.pc_stops for a in stop.addresses] or None,
                guest_breaks=True,
                console_output=True,
            )
            self.session._take_console()
            stopped = self._stopped_by(reason, value) or self._periodic()
            if stopped is not None:
                return stopped

    def _stopped(self, by: Stop, **details: Any) -> Stopped:
        return Stopped(
            by=by, cycle=self.sim.cycle, instructions=self.sim.instructions_retired, **details
        )

    def _reached(self) -> Optional[Stopped]:
        """A stop that holds without running further, if any."""
        matched = self.session._match_console(self.consoles)
        if matched is not None:
            return self._stopped(matched[0], match=matched[1])
        for target, stop in self.instruction_targets:
            if self.sim.instructions_retired >= target:
                return self._stopped(stop)
        for target, stop in self.cycle_targets:
            if self.sim.cycle >= target:
                return self._stopped(stop)
        return None

    def _stopped_by(self, reason: str, value: Optional[int]) -> Optional[Stopped]:
        """The stop ``run_to``'s ``reason`` means, if it is one of ours."""
        if reason == "exit":
            return self._stopped(self.exit, exit_code=value)
        if reason == "pc":
            assert value is not None
            pc = self.sim.harts[value].pc
            stop = next(s for s in self.pc_stops if pc in s.addresses)
            return self._stopped(stop, hart=value)
        if reason == "break":
            for marker in self.markers:
                if marker.label is None or marker.label == value:
                    return self._stopped(marker, label=value)
        return None

    def _cycles_to_run(self) -> Optional[int]:
        horizons = [target for target, _ in self.cycle_targets]
        horizons += [check for check, _ in self.whens]
        if self.next_every is not None:
            horizons.append(self.next_every)
        now = self.sim.cycle
        if self.session._progress:
            horizons.append(now + _PROGRESS_CYCLES)
        if not horizons:
            return None
        return max(1, min(horizons) - now)

    def _instructions_to_run(self) -> Optional[int]:
        if not self.instruction_targets:
            return None
        remaining = min(target for target, _ in self.instruction_targets)
        return max(1, remaining - self.sim.instructions_retired)

    def _periodic(self) -> Optional[Stopped]:
        """Runs the callbacks that are due; a :class:`When` that holds
        stops the run."""
        now = self.sim.cycle
        if self.next_every is not None and now >= self.next_every:
            assert self.on_every is not None and self.every is not None
            self.on_every(self.session)
            self.next_every += self.every
        for entry in self.whens:
            check, when = entry
            if now >= check:
                if when.predicate(self.session):
                    return self._stopped(when)
                entry[0] = check + when.every
        self._report()
        return None

    def _report(self) -> None:
        if not self.session._progress:
            return
        host = time.perf_counter()
        if host - self.last_report < _PROGRESS_SECONDS:
            return
        self.last_report = host
        elapsed = host - self.started
        cycles = self.sim.cycle - self.start_cycle
        phase = self.session._phase or "run"
        self.session._say(
            f"{phase}: {self.sim.cycle:,} cycles, {self.sim.instructions_retired:,} instructions, "
            f"{cycles / elapsed / 1e3:,.0f}k cycles/s"
        )
