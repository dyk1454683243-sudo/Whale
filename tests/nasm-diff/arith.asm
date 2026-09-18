section .text
global _start
_start:
    mov eax, 1
    mov ebx, eax
    add ebx, 10
    sub ebx, 5
    xor ecx, ecx
    cmp ebx, 15
    ret
