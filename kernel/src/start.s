/* Punto de entrada del kernel de rockyOS (64 bits, modo largo).
 * El bootloader ya ha identidad-mapeado la memoria y nos salta a 0x10000. */

.section .text.boot, "ax"
.code64
.global _start
_start:
    cli
    cld
    mov     rsp, 0x400000
    xor     rbp, rbp

    /* Pone a cero .bss. */
    lea     rdi, [rip + __bss_start]
    lea     rcx, [rip + __bss_end]
    sub     rcx, rdi
    xor     eax, eax
    rep     stosb

    call    rust_main
.Lhang:
    cli
    hlt
    jmp     .Lhang
