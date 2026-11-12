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
    . = KERNEL_BASE;
    __kernel_start = .;

    .multiboot2 ALIGN(8) :
    {
        KEEP(*(.multiboot2))
    } :text

    .text ALIGN(PAGE_SIZE) :
    {
        *(.text.boot)
        *(.text .text.*)
    } :text

    .rodata ALIGN(PAGE_SIZE) :
    {
        *(.rodata .rodata.*)
    } :rodata

    .data ALIGN(PAGE_SIZE) :
    {
        *(.data .data.*)
        *(.got .got.*)
    } :data

    .bss ALIGN(PAGE_SIZE) (NOLOAD) :
    {
        *(COMMON)
        *(.bss .bss.*)
    } :data
    __kernel_end = ALIGN(PAGE_SIZE);

    /DISCARD/ :
    {
        *(.eh_frame*)
        *(.comment)
        *(.note*)
    }
}

ASSERT(__kernel_end <= LAPIC_DEFAULT, "kernel image exceeds the bootstrap RAM window")
