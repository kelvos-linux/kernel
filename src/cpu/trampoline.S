.section .ap_trampoline,"ax"
.balign 16
.code16

.global ap_trampoline_start
.global ap_trampoline_end
.global ap_trampoline_data
.global ap_trampoline_gdtr
.global ap_trampoline_gdt

.set AP_GDTR_OFFSET, ap_trampoline_gdtr - ap_trampoline_start
.set AP_DATA_OFFSET, ap_trampoline_data - ap_trampoline_start
.set AP_PM_OFFSET, ap_trampoline_pm_entry - ap_trampoline_start
.set AP_LM_OFFSET, ap_trampoline_lm_entry - ap_trampoline_start
.set AP_DATA_PHYS, 0x8000 + AP_DATA_OFFSET

ap_trampoline_start:
    cli
    cld
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x8ff0, %sp
    lgdt %cs:AP_GDTR_OFFSET
    movl %cs:AP_DATA_OFFSET, %eax
    movl %eax, %cr3

    movl %cr4, %eax
    orl $0x620, %eax
    movl %eax, %cr4

    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    pushl $0x08
    pushl $(0x8000 + AP_PM_OFFSET)
    .byte 0x66, 0xcb

.code32
ap_trampoline_pm_entry:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss

    movl $0xc0000080, %ecx
    rdmsr
    orl $0x900, %eax
    wrmsr

    movl %cr0, %eax
    andl $0xfffffffb, %eax
    orl $0x80010002, %eax
    movl %eax, %cr0
    pushl $0x18
    pushl $(0x8000 + AP_LM_OFFSET)
    lret

.code64
ap_trampoline_lm_entry:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss

    movq AP_DATA_PHYS+16, %rsp
    andq $-16, %rsp
    movq AP_DATA_PHYS+24, %rax
    movl $0xc0000101, %ecx
    movq %rax, %rdx
    shrq $32, %rdx
    wrmsr

    movl AP_DATA_PHYS+4, %edi
    movq AP_DATA_PHYS+32, %rax
    call *%rax
2:
    cli
    hlt
    jmp 2b

.balign 8
ap_trampoline_gdtr:
    .word ap_trampoline_gdt_end - ap_trampoline_gdt - 1
    .long 0

ap_trampoline_gdt:
    .quad 0
    .quad 0x00cf9a000000ffff
    .quad 0x00cf92000000ffff
    .quad 0x00af9a000000ffff
ap_trampoline_gdt_end:

.balign 8
ap_trampoline_data:
    .long 0
    .long 0
    .long 0
    .long 0
    .quad 0
    .quad 0
    .quad 0

ap_trampoline_end:
