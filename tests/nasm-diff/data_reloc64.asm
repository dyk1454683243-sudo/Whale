section .data
global ptr64
extern extdata
ptr64:
    dq extdata + 8
