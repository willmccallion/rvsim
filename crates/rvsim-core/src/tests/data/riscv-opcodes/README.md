# riscv-opcodes data

`instr_dict.json` and `arg_lut.csv` come from
[riscv-opcodes](https://github.com/riscv/riscv-opcodes) at commit
`0a7bd769454e40675f3920edd71b02bb016a3c8d`, under its BSD-3-Clause
`LICENSE`. They list every ratified instruction's encoding and operand
fields, and each field's bit position.

`instr_dict.json` was generated, ratified extensions only, with

    PYTHONPATH=src python3 -m riscv_opcodes "rv*"

run in a checkout of that commit; `arg_lut.csv` is copied as it is.
`tests/unit/exec/opcode_operands.rs` checks decode against them.
