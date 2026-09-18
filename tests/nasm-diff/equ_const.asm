section .text
global _start
VAL equ 5
_start:
    mov eax, VAL + 3
    add eax, VAL - 1
    ret
