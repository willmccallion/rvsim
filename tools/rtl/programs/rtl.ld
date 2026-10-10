/* software/libc/user.ld with the HTIF words on their own page and a stack
 * top, for programs that run unchanged on rvsim and on a Chipyard core. */
ENTRY(_start)

SECTIONS {
  . = 0x80000000;

  .text : {
    *(.text .text.*)
  }

  .rodata : ALIGN(8) {
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
  }

  .data : ALIGN(8) {
    PROVIDE(__global_pointer$ = . + 0x800);
    *(.data .data.*)
    *(.sdata .sdata.*)
  }

  .bss : ALIGN(8) {
    _bss_start = .;
    *(.bss .bss.*)
    *(.sbss .sbss.*)
    *(COMMON)
    . = ALIGN(8);
    _bss_end = .;
  }

  .tohost : ALIGN(0x1000) {
    *(.tohost)
  }

  . = ALIGN(8);
  heap_top = .;
  _end = .;

  /* Where rvsim's direct mode also puts the stack: 16 MiB above the base. */
  _stack_top = 0x81000000;
}
