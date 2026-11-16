/* ------------------------------------------------------------------------
 * rockyOS - sector de arranque del disquete (BIOS, 16 bits)
 * ------------------------------------------------------------------------
 * Este sector ya no carga el kernel: lo que hace es cargar el **segundo
 * tramo** (`bootusb2.bin`, en el LBA 1) a 0x8000 y saltar ahi. El segundo
 * tramo es el que carga el kernel y el que entra en modo largo.
 *
 * Antes este sector hacia todo: leer el kernel, copiarlo a 1 MiB, montar la
 * paginacion y saltar. Con la ventana de carga ampliada a tres tramos (ver
 * `carga_kernel` en `bootusb2.s`) ese trabajo ya no cabe en 512 bytes, asi
 * que el reparto en dos tramos es el mismo que usa el pendrive: el sector
 * de arranque queda con mucho sitio y, si la BIOS no lee el disco, se puede
 * decir por pantalla en vez de saltar a memoria a ejecutar lo que hubiera
 * ahi.
 *
 * El kernel empieza en el LBA 5, igual que en el pendrive: el LBA 0 es este
 * sector y los LBA 1..4 el segundo tramo (ver `USB_KERNEL_LBA` en xtask).
 * ------------------------------------------------------------------------ */

.section .text.boot, "ax"

.code16
.global _start
_start:
    cli
    xor     ax, ax
    mov     ds, ax
    mov     es, ax
    mov     ss, ax
    mov     sp, 0x7C00
    cld
    mov     [boot_drive], dl
    mov     byte ptr [0x7F00], dl   /* la unidad que dio la BIOS, para el 2o tramo */
    call    serie_init

    /* Segundo tramo: los LBA 1..4 a la direccion lineal 0x8000, que es
     * donde esta enlazado `bootusb2`. */
    mov     dl, [boot_drive]
    lea     si, [dap]
    mov     ax, 0x4200
    int     0x13
    jnc     .Lsalta

    /* Fallback para BIOS sin INT 13h Extensions: el segundo tramo empieza en
     * el sector 2 del primer cilindro (los LBA 1..4). Se carga con CHS en
     * 0x0000:0x8000. Tambien se reinicia el controlador. */
    xor     ah, ah
    mov     dl, [boot_drive]
    int     0x13
    mov     ax, 0x0204
    xor     bx, bx
    mov     es, bx
    mov     bx, 0x8000
    xor     cx, cx
    mov     cl, 2
    xor     dh, dh
    mov     dl, [boot_drive]
    int     0x13
    jnc     .Lsalta

    /* La BIOS no ha leido. Antes se seguia adelante y se saltaba a ejecutar
     * lo que hubiera en 0x8000: en un PC real eso se ve como un reinicio en
     * bucle y sin mensaje. Ahora se dice lo que ha contestado la BIOS. */
    mov     byte ptr [boot_err], ah
    lea     si, [msg_lectura]
    call    imprime
    mov     al, [boot_err]
    call    hex8
    lea     si, [msg_unidad]
    call    imprime
    mov     al, [boot_drive]
    call    hex8
    lea     si, [msg_punto]
    call    imprime
.Lalto:
    cli
    hlt
    jmp     .Lalto

.Lsalta:
    .byte   0xEA                    /* jmp 0x0000:0x8000 (segundo tramo) */
    .word   0x8000
    .word   0x0000

/* ---- Puerto de serie (COM1) ----
 * En un PC de verdad nadie lee esto, pero al emular deja el rastro del
 * arranque en un fichero: asi se puede comprobar sin mirar la pantalla. */
serie_init:
    mov     dx, 0x3FB
    mov     al, 0x80
    out     dx, al                  /* DLAB = 1 */
    mov     dx, 0x3F8
    mov     al, 1
    out     dx, al                  /* divisor 1: 115200 bps */
    mov     dx, 0x3F9
    xor     al, al
    out     dx, al
    mov     dx, 0x3FB
    mov     al, 3
    out     dx, al                  /* 8N1, DLAB = 0 */
    mov     dx, 0x3FA
    mov     al, 0xC7
    out     dx, al                  /* FIFO encendida y limpia */
    mov     dx, 0x3FC
    mov     al, 0x0B
    out     dx, al                  /* DTR, RTS y OUT2 */
    ret

putc:                               /* AL = caracter (pantalla y serie) */
    push    ax
    push    bx
    push    dx
    mov     ah, 0x0E
    mov     bx, 0x0007              /* pagina 0, atributo normal */
    int     0x10
    mov     dx, 0x3F8
    out     dx, al
    pop     dx
    pop     bx
    pop     ax
    ret

imprime:                            /* DS:SI = cadena acabada en cero */
    push    ax
    push    si
.Lbucle:
    lodsb
    test    al, al
    jz      .Lfin
    call    putc
    jmp     .Lbucle
.Lfin:
    pop     si
    pop     ax
    ret

hex8:                               /* AL = byte -> dos digitos hexadecimales */
    push    ax
    mov     ah, al
    shr     al, 4
    call    hexd
    mov     al, ah
    call    hexd
    pop     ax
    ret

hexd:                               /* AL = 0..15 -> un digito */
    and     al, 0x0F
    cmp     al, 10
    jb      .Ldigito
    add     al, 'A' - 10
    jmp     putc
.Ldigito:
    add     al, '0'
    jmp     putc

/* ---- Datos ---- */
.align 2
boot_drive:     .byte 0
                .byte 0                 /* relleno: la DAP queda alineada */
boot_err:       .byte 0                 /* codigo de error de la BIOS */
                .byte 0                 /* relleno: la DAP queda alineada */

/* Mensajes de error (pantalla y COM1). */
msg_lectura:    .asciz "rocky: el disquete no arranca (error 0x"
msg_unidad:     .asciz ", unidad 0x"
msg_punto:      .asciz ").\r\n"

/* Disk Address Packet (INT 13h AH=42h): LBA 1, 4 sectores, 0x0000:0x8000
 * (la direccion lineal 0x8000, que es donde el segundo tramo esta enlazado).
 * El kernel empieza justo detras (LBA 5); ver `USB_KERNEL_LBA` en xtask. */
dap:            .byte 0x10
                .byte 0x00
                .word 4
                .word 0x0000
                .word 0x0800
                .long 1
                .long 0
