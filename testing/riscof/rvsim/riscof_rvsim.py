"""rvsim DUT plugin for riscof.

Compiles arch-tests and runs them on rvsim, extracting the signature
region from simulator memory after execution.
"""

import os
import sys
import shutil
import logging

import riscof.utils as utils
from riscof.pluginTemplate import pluginTemplate

logger = logging.getLogger()

TOOLCHAIN_PREFIX = "riscv64-elf-"


class rvsim(pluginTemplate):
    __model__ = "rvsim"
    __version__ = "1.0.0"

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)

        config = kwargs.get('config')
        if config is None:
            raise SystemExit("rvsim plugin: missing config")

        self.num_jobs = str(config.get('jobs', 1))
        self.pluginpath = os.path.abspath(config['pluginpath'])
        self.isa_spec = os.path.abspath(config['ispec'])
        self.platform_spec = os.path.abspath(config['pspec'])

        # Path to the rvsim_run.py helper script
        self.run_script = os.path.join(self.pluginpath, 'rvsim_run.py')

        # Use the same Python that's running riscof (the venv Python)
        self.python = sys.executable

    def initialise(self, suite, work_dir, archtest_env):
        self.work_dir = work_dir
        self.suite = suite

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
        self.mabi = 'lp64d' if self.xlen == '64' else 'ilp32d'

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

            elf = 'dut.elf'
            sig_file = os.path.join(test_dir, self.name[:-1] + ".signature")

            compile_macros = ' -D' + " -D".join(testentry['macros'])

            march = testentry['isa'].lower()
            cmd = self.compile_cmd.format(
                march, self.mabi, test, elf, compile_macros
            )

            simcmd = f'{self.python} {self.run_script} {elf} {sig_file}'

            execute = f'@cd {test_dir}; {cmd}; {simcmd};'
            make.add_target(execute)

        make.execute_all(self.work_dir)
