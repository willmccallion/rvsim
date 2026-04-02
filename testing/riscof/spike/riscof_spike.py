"""Spike reference plugin for riscof.

Compiles arch-tests and runs them on Spike, using its built-in +signature
flag to dump the signature region to a file.
"""

import os
import shutil
import logging

import riscof.utils as utils
from riscof.pluginTemplate import pluginTemplate

logger = logging.getLogger()

TOOLCHAIN_PREFIX = "riscv64-elf-"


class spike(pluginTemplate):
    __model__ = "spike"
    __version__ = "1.1.0"

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

        config = kwargs.get('config')
        if config is None:
            raise SystemExit("spike plugin: missing config")

        self.spike_exe = os.path.join(
            config.get('PATH', ''), 'spike'
        )
        self.num_jobs = str(config.get('jobs', 1))
        self.pluginpath = os.path.abspath(config['pluginpath'])
        self.isa_spec = os.path.abspath(config['ispec'])
        self.platform_spec = os.path.abspath(config['pspec'])

    def initialise(self, suite, work_dir, archtest_env):
        self.work_dir = work_dir
        self.suite = suite

        if shutil.which(self.spike_exe) is None:
            logger.error(
                f"{self.spike_exe}: not found. Install spike to use as reference."
            )
            raise SystemExit(1)

        self.compile_cmd = (
            f'{TOOLCHAIN_PREFIX}gcc -march={{0}} -mabi={{1}} '
            f'-static -mcmodel=medany -fvisibility=hidden -nostdlib -nostartfiles '
            f'-T {self.pluginpath}/env/link.ld '
            f'-I {self.pluginpath}/env/ '
            f'-I {archtest_env} '
            f'{{2}} -o {{3}} {{4}}'
        )

    def build(self, isa_yaml, platform_yaml):
        ispec = utils.load_yaml(isa_yaml)['hart0']
        self.xlen = '64' if 64 in ispec['supported_xlen'] else '32'

        # Build the ISA string for spike --isa flag.
        # Start with the base single-letter extensions.
        self.isa = 'rv' + self.xlen
        for ext in 'iemafdqlcbjktpvnshu':
            if ext.upper() in ispec['ISA']:
                self.isa += ext

        # Append Z-extensions that Spike understands (skip Zicsr/Zifencei
        # which Spike enables implicitly and may not parse via --isa).
        import re
        z_exts = re.findall(r'Z[a-z]+', ispec['ISA'])
        for z in z_exts:
            if z.lower() in ('zicsr', 'zifencei'):
                continue
            self.isa += '_' + z.lower()

    @staticmethod
    def _mabi_for_march(march, xlen):
        """Pick the correct ABI based on which FP extensions are in march."""
        base = march.split('_')[0].upper()
        if xlen == '64':
            if 'D' in base:
                return 'lp64d'
            if 'F' in base:
                return 'lp64f'
            return 'lp64'
        else:
            if 'D' in base:
                return 'ilp32d'
            if 'F' in base:
                return 'ilp32f'
            return 'ilp32'

    def runTests(self, testList):
        if os.path.exists(self.work_dir + "/Makefile." + self.name[:-1]):
            os.remove(self.work_dir + "/Makefile." + self.name[:-1])

        make = utils.makeUtil(
            makefilePath=os.path.join(self.work_dir, "Makefile." + self.name[:-1])
        )
        make.makeCommand = 'make -k -j' + self.num_jobs

        for testname in testList:
            testentry = testList[testname]
            test = testentry['test_path']
            test_dir = testentry['work_dir']

            elf = 'ref.elf'
            sig_file = os.path.join(test_dir, self.name[:-1] + ".signature")

            compile_macros = ' -D' + " -D".join(testentry['macros'])

            march = testentry['isa'].lower()
            mabi = self._mabi_for_march(march, self.xlen)
            cmd = self.compile_cmd.format(
                march, mabi, test, elf, compile_macros
            )

            # Spike 1.x doesn't understand underscore Z-extension syntax
            # (e.g. rv64ifd_zicsr). Use the full base ISA which implicitly
            # enables all sub-extensions Spike supports.
            simcmd = (
                f'{self.spike_exe} --isa={self.isa} '
                f'+signature={sig_file} +signature-granularity=4 {elf}'
            )

            execute = f'@cd {test_dir}; {cmd}; {simcmd};'
            make.add_target(execute)

        make.execute_all(self.work_dir)
