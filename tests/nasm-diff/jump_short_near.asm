section .text
global _start

_start:
    jmp far_target
    dq 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    dq 0, 0, 0, 0, 0, 0, 0, 0, 0, 0

near_src:
    je near_dst
    nop
near_dst:
    loop near_src

far_target:
    ret
