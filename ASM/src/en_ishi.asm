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

; Actor instance in s0
; Put the color index in 0x34(sp)
; 0 for silver boulder. 2 for gold boulder
EnIshi_SetKakeraColorIndex:
    lh      t2, 0x1C(s0) ; Read the actor spawn variable
    andi    t2, t2, 0x0F ; Get the lower 4 bits
    li      t0, 0x03 ; Gold
    bnel    t2, t0, @@return
    sw      r0, 0x34(sp)
@@gold:
    li      t2, 0x02
    sw      t2, 0x34(sp)
@@return:
    jr      ra
    nop