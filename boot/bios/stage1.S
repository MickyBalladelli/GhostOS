.code16
.section .text
.global _start

.set STAGE2_SEGMENT, 0x0800
.set STAGE2_SECTORS, 16

_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x7c00, %sp
    sti
    movb %dl, boot_drive

    movw $disk_packet, %si
    movb $0x42, %ah
    movb boot_drive, %dl
    int $0x13
    jc disk_error

    movb boot_drive, %dl
    ljmp $STAGE2_SEGMENT, $0

disk_error:
    movw $disk_error_message, %si
print:
    lodsb
    testb %al, %al
    jz halt
    movb $0x0e, %ah
    movw $0x0007, %bx
    int $0x10
    jmp print

halt:
    cli
    hlt
    jmp halt

boot_drive:
    .byte 0

disk_packet:
    .byte 0x10, 0
    .word STAGE2_SECTORS
    .word 0
    .word STAGE2_SEGMENT
    .quad 1

disk_error_message:
    .asciz "SynOS: stage2 read failed"

.org 510
.word 0xaa55

