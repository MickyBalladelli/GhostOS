ENTRY(_start)

SECTIONS
{
    . = 0x0000008000000000;

    .text : ALIGN(4096) {
        KEEP(*(.text._start))
        *(.text .text.*)
    }

    .rodata : ALIGN(16) {
        *(.rodata .rodata.*)
        *(.lrodata .lrodata.*)
    }

    .stack_guard 0x0000008000003ff8 : {
        KEEP(*(.stack_guard))
    }

    /DISCARD/ : {
        *(.comment .note* .eh_frame*)
    }
}
