.code16
.section .text
.global _start

.set BOOT_INFO, 0x7000
.set MEMORY_REGIONS, BOOT_INFO + 72
.set KERNEL_SEGMENT, 0x1000
.set KERNEL_ADDRESS, 0x100000
.set KERNEL_SECTORS, 512
.set STAGE2_SECTORS, 16

_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x7000, %sp
    movb %dl, boot_drive

    call enable_a20
    call build_boot_info
    call load_kernel
    call enter_long_mode

halt:
    cli
    hlt
    jmp halt

enable_a20:
    inb $0x92, %al
    orb $0x02, %al
    andb $0xfe, %al
    outb %al, $0x92
    ret

build_boot_info:
    movl $0x53424f4f, BOOT_INFO
    movl $0x53594e4f, BOOT_INFO + 4
    movl $1, BOOT_INFO + 8
    movl $1, BOOT_INFO + 12
    movl $0, BOOT_INFO + 16
    movl $0, BOOT_INFO + 20
    movl $0, BOOT_INFO + 24
    movl $0, BOOT_INFO + 28
    movl $0, BOOT_INFO + 64
    movl $0, BOOT_INFO + 68

    xorl %ebx, %ebx
    movw $MEMORY_REGIONS, %di
    xorl %ebp, %ebp

e820_next:
    cmpl $128, %ebp
    jae e820_done
    movl $0xe820, %eax
    movl $0x534d4150, %edx
    movl $24, %ecx
    movl $1, 20(%di)
    int $0x15
    jc e820_done
    cmpl $0x534d4150, %eax
    jne e820_done
    testl %ecx, %ecx
    jz e820_continue

    cmpl $1, 16(%di)
    je e820_kind_ready
    cmpl $4, 16(%di)
    jbe e820_kind_ready
    movl $2, 16(%di)

e820_kind_ready:
    addw $24, %di
    incl %ebp

e820_continue:
    testl %ebx, %ebx
    jnz e820_next

e820_done:
    movl %ebp, BOOT_INFO + 64
    ret

load_kernel:
    movw $KERNEL_SEGMENT, destination_segment
    movw $KERNEL_SECTORS, sectors_remaining
    movl $(1 + STAGE2_SECTORS), disk_lba

read_chunk:
    cmpw $0, sectors_remaining
    je kernel_loaded
    movw sectors_remaining, %ax
    cmpw $127, %ax
    jbe chunk_size_ready
    movw $127, %ax

chunk_size_ready:
    movw %ax, disk_count
    movw destination_segment, %dx
    movw %dx, disk_segment
    movw $disk_packet, %si
    movb $0x42, %ah
    movb boot_drive, %dl
    int $0x13
    jc disk_failure

    movw disk_count, %ax
    subw %ax, sectors_remaining
    shlw $5, %ax
    addw %ax, destination_segment
    movzwl disk_count, %eax
    addl %eax, disk_lba
    jmp read_chunk

kernel_loaded:
    ret

disk_failure:
    movb $'D', %al
    movw $0x3f8, %dx
    outb %al, %dx
    jmp halt

enter_long_mode:
    lgdt gdt_pointer
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl $0x08, $protected_mode

.code32
protected_mode:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl $0x90000, %esp

    movl $KERNEL_SEGMENT * 16, %esi
    movl $KERNEL_ADDRESS, %edi
    movl $(KERNEL_SECTORS * 512 / 4), %ecx
    rep movsl

    xorl %eax, %eax
    movl $0x1000, %edi
    movl $(3 * 4096 / 4), %ecx
    rep stosl

    movl $0x2003, 0x1000
    movl $0x3003, 0x2000
    movl $0x0083, %eax
    movl $0x3000, %edi
    movl $512, %ecx

map_two_mib:
    movl %eax, (%edi)
    addl $0x200000, %eax
    addl $8, %edi
    loop map_two_mib

    movl $0x1000, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $(1 << 5), %eax
    movl %eax, %cr4

    movl $0xc0000080, %ecx
    rdmsr
    orl $(1 << 8), %eax
    wrmsr

    movl %cr0, %eax
    orl $(1 << 31), %eax
    movl %eax, %cr0
    ljmpl $0x18, $long_mode

.code64
long_mode:
    movw $0x20, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movq $0x90000, %rsp
    xorq %rbp, %rbp
    movq $BOOT_INFO, %rdi
    movq $KERNEL_ADDRESS, %rax
    jmp *%rax

.code16
.align 8
gdt:
    .quad 0
    .quad 0x00cf9a000000ffff
    .quad 0x00cf92000000ffff
    .quad 0x00af9a000000ffff
    .quad 0x00af92000000ffff
gdt_end:

gdt_pointer:
    .word gdt_end - gdt - 1
    .long gdt

boot_drive:
    .byte 0
sectors_remaining:
    .word 0
destination_segment:
    .word 0

.align 4
disk_packet:
    .byte 0x10, 0
disk_count:
    .word 0
    .word 0
disk_segment:
    .word 0
disk_lba:
    .quad 0

