"""A debugger attaches to a run with GDB's remote protocol."""

import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest

from rvsim import Config, Simulator
from tests.python.test_se_mode import (
    CODE_OFFSET,
    COUNT_THEN_EXIT,
    EXIT_WITH_ARGC,
    LOAD_ADDRESS,
    user_elf,
)

ENTRY = LOAD_ADDRESS + CODE_OFFSET
# The `addi a0, zero, 9` after COUNT_THEN_EXIT's loop.
AFTER_THE_LOOP = ENTRY + 16


def free_port() -> int:
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class Debugger:
    """The debugger's side of the protocol, acknowledging until no-ack mode."""

    def __init__(self, port: int):
        deadline = time.monotonic() + 60
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=60)
                break
            except OSError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.02)
        self.acks = True
        assert "QStartNoAckMode+" in self.ask("qSupported")
        assert self.ask("QStartNoAckMode") == "OK"
        self.acks = False

    def ask(self, payload: str) -> str:
        data = payload.encode()
        self.sock.sendall(b"$" + data + b"#%02x" % (sum(data) % 256))
        reply = bytearray()
        while True:
            byte = self.sock.recv(1)
            if not byte:
                raise EOFError("the stub hung up")
            if byte in (b"+", b"-") and not reply:
                continue
            if byte == b"$":
                reply.clear()
            elif byte == b"#":
                self.sock.recv(2)
                if self.acks:
                    self.sock.sendall(b"+")
                return reply.decode()
            else:
                reply += byte

    def close(self) -> None:
        self.sock.close()


class RemoteDebugging(unittest.TestCase):
    def test_a_breakpoint_stops_the_run_and_the_exit_ends_the_session(self):
        cpu = Simulator(Config(), elf_data=user_elf(COUNT_THEN_EXIT), args=["prog"])
        port = free_port()
        seen: list[str] = []

        def script():
            gdb = Debugger(port)
            seen.append(gdb.ask(f"Z0,{AFTER_THE_LOOP:x},4"))
            seen.append(gdb.ask("c"))
            seen.append(gdb.ask("p5"))
            seen.append(gdb.ask("c"))
            gdb.close()

        client = threading.Thread(target=script)
        client.start()
        outcome = cpu.gdb(port)
        client.join()

        self.assertEqual(outcome, ("exit", 9))
        self.assertEqual(seen[0], "OK")
        self.assertTrue(seen[1].startswith("T05thread:1;"), seen[1])
        self.assertEqual(seen[2], "2c01000000000000", "t0 counted to 300")
        self.assertEqual(seen[3], "W09")

    def test_the_command_line_waits_for_gdb_and_goes_on_after_a_detach(self):
        port = free_port()
        with tempfile.TemporaryDirectory() as tmp:
            path = os.path.join(tmp, "prog.elf")
            with open(path, "wb") as f:
                f.write(user_elf(EXIT_WITH_ARGC))

            def detach():
                gdb = Debugger(port)
                gdb.ask("D")
                gdb.close()

            client = threading.Thread(target=detach)
            client.start()
            result = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "rvsim",
                    path,
                    "--gdb",
                    str(port),
                    "--quiet",
                    "a",
                    "b",
                ],
                capture_output=True,
                text=True,
                timeout=600,
                check=False,
            )
            client.join()

        self.assertIn(f"waiting for gdb on 127.0.0.1:{port}", result.stderr)
        self.assertEqual(result.returncode, 3, result.stderr)
