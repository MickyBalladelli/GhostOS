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

    /DISCARD/ : {
        *(.comment .note* .eh_frame*)
    }
}
