#ifndef FDT_H
#define FDT_H

/* The number of cpu nodes in the flattened device tree at fdt, which the
 * boot ROM hands a bare-metal program in a1; 1 when it is not a tree. */
unsigned long fdt_cpu_count(const void *fdt);

#endif
