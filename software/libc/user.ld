ENTRY(_start)

SECTIONS {
  /* User programs are loaded at 0x80000000 for direct/bare-metal execution */
  /* For OS mode, the kernel occupies 0x80000000-0x80200000 */
  . = 0x80000000;

  .text : {
    *(.text .text.*)
  }

  /* Rodata needs 8-byte alignment for double-precision FP constants */
  .rodata : ALIGN(8) {
    . = ALIGN(8);
    *(.rodata .rodata.*)
    *(.srodata .srodata.*)
  }

  .data : ALIGN(8) {
    . = ALIGN(8);
    /* Define gp for relaxation optimizations */
    PROVIDE(__global_pointer$ = . + 0x800);
    *(.data .data.*)
    *(.sdata .sdata.*)
  }

  /* BSS Section: Must be zeroed by crt0.s */
  .tohost : ALIGN(16) {
    *(.tohost)
  }

  .bss : ALIGN(8) {
    . = ALIGN(8);
    _bss_start = .;
    *(.bss .bss.*)
    *(.sbss .sbss.*)
    *(COMMON)
    . = ALIGN(8);
    _bss_end = .;
  }

  /* Heap starts after BSS */
  . = ALIGN(8);
  heap_top = .;
  _end = .;

  /* Where rvsim's direct mode also puts the stack: 16 MiB above the base. */
  _stack_top = 0x81000000;
}
