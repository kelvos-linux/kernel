
OUTPUT_FORMAT(elf64-x86-64)
ENTRY(_start)

PHDRS
{
    text PT_LOAD FLAGS(5);
    rodata PT_LOAD FLAGS(4);
    data PT_LOAD FLAGS(6);
}

SECTIONS
{
    . = 1M;
    __kernel_start = .;

    .multiboot2 ALIGN(8) :
    {
        KEEP(*(.multiboot2))
    } :text

    .text ALIGN(4K) :
    {
        *(.text.boot)
        *(.text .text.*)
    } :text

    .rodata ALIGN(4K) :
    {
        *(.rodata .rodata.*)
    } :rodata

    .data ALIGN(4K) :
    {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss ALIGN(4K) (NOLOAD) :
    {
        *(COMMON)
        *(.bss .bss.*)
    } :data
    __kernel_end = ALIGN(4K);

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
    }
}
