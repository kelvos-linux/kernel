
.section .multiboot2,"a"
.balign 8

header_start:
    .long 0xe85250d6
    .long 0
    .long header_end - header_start
    .long -(0xe85250d6 + (header_end - header_start))

    # Request the ACPI 2.0 RSDP tag so Rust can enumerate processors from MADT.
    .short 1
    .short 0
    .long 12
    .long 15
    .balign 8

    # Required end tag
    .short 0
    .short 0
    .long 8

header_end:


.section .text.boot,"ax"
.code32

.global _start
.extern rust_entry

_start:
    cli
    cld

    # Save Multiboot2 registers
    movl $stack_top, %esp
    movl %eax, boot_magic
    movl %ebx, boot_info_ptr

    # Load our own GDT
    lgdt gdt64_ptr

    # Clear page tables
    movl $pml4_table, %edi
    xorl %eax, %eax
    movl $(6144), %ecx
    rep stosl

    # PML4[0] -> PDPT
    movl $pdpt_table, %eax
    orl $3, %eax
    movl %eax, pml4_table

    # PDPT entries -> four page directories
    movl $pd_tables, %eax
    orl $3, %eax
    movl %eax, pdpt_table

    movl $(pd_tables + 4096), %eax
    orl $3, %eax
    movl %eax, pdpt_table + 8

    movl $(pd_tables + 8192), %eax
    orl $3, %eax
    movl %eax, pdpt_table + 16

    movl $(pd_tables + 12288), %eax
    orl $3, %eax
    movl %eax, pdpt_table + 24

    # Identity map the first 4 GiB using 2 MiB pages
    movl $pd_tables, %edi
    xorl %ecx, %ecx

map_pages:
    movl %ecx, %eax
    shll $21, %eax
    orl $0x83, %eax

    movl %eax, (%edi,%ecx,8)
    movl $0, 4(%edi,%ecx,8)

    incl %ecx
    cmpl $2048, %ecx
    jne map_pages

    # Enable PAE and SSE support
    movl %cr4, %eax
    andl $0xfffaffff, %eax # Disable FSGSBASE and OSXSAVE
    orl $0x620, %eax
    movl %eax, %cr4

    # Load page table
    movl $pml4_table, %eax
    movl %eax, %cr3

    # Enable long mode in EFER
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x900, %eax
    wrmsr

    # Enable paging and write protection
    movl %cr0, %eax
    andl $0xFFFFFFF3, %eax
    orl $0x80010002, %eax
    movl %eax, %cr0

    # Switch to 64-bit code segment
    ljmp $0x08, $long_mode_start


.code64

long_mode_start:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss

    # Set up 64-bit stack
    movabsq $stack_top, %rsp
    xorq %rbp, %rbp

    # Pass Multiboot2 magic and information address
    movl boot_magic(%rip), %edi
    movl boot_info_ptr(%rip), %esi

    # Call Rust kernel
    call rust_entry

halt:
    cli
    hlt
    jmp halt


.section .rodata
.balign 8

gdt64:
    .quad 0x0000000000000000
    .quad 0x00AF9A000000FFFF
    .quad 0x00CF92000000FFFF

gdt64_end:

gdt64_ptr:
    .word gdt64_end - gdt64 - 1
    .long gdt64


.section .bss,"aw",@nobits
.balign 4096

pml4_table:
    .skip 4096

pdpt_table:
    .skip 4096

pd_tables:
    .skip 16384

.balign 16

stack_bottom:
    .skip 16384

stack_top:

boot_magic:
    .skip 4

boot_info_ptr:
    .skip 4
