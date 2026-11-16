/* Linker script comun de los ejecutables de usuario de Rocky. */
ENTRY(_start)

SECTIONS
{
    . = 0x0000008000000000;

    .text : { *(.text .text.*) }
    .rodata : ALIGN(16) { *(.rodata .rodata.*) }
    .data : ALIGN(16) { *(.data .data.*) }
    .bss : ALIGN(16) { *(.bss .bss.*) *(COMMON) }

    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
        *(.eh_frame)
        *(.eh_frame_hdr)
    }
}
