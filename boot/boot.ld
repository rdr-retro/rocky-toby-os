/* Script de enlazado del sector de arranque de rocky.
 * El sector vive en 0x7C00, mide exactamente 512 bytes y termina en 0xAA55. */
ENTRY(_start)

SECTIONS
{
    . = 0x7C00;

    .text : {
        *(.text.boot)
        *(.text .text.*)
    }

    .rodata : { *(.rodata .rodata.*) }
    .data   : { *(.data .data.*) }

    . = 0x7DFE;
    .bootsig : { SHORT(0xAA55) }

    /DISCARD/ : {
        *(.comment)
        *(.note .note.*)
        *(.eh_frame .eh_frame.*)
        *(.bss .bss.*)
        *(COMMON)
    }
}

ASSERT(. <= 0x7E00, "el sector de arranque excede 512 bytes")
