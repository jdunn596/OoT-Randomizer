EnIshi_SetPrimColor:
    lw      a0, 0x28(sp) ; Get the actor pointer off the stack
    lh      a0, 0x1C(a0) ; Read the actor spawn variable
    andi    a0, a0, 0x0F ; Get the lower 4 bits
    li      at, 0x01 ; Check if it's a regular silver boulder
    beq     a0, at, @@return
    nop
    li      t2, 0xFFD500FF
@@return:
    jr      ra
    sw      t2, 0x04(v1) ; Set the color