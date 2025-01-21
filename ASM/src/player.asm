; Hack called from player overlay to check if it can lift EnIshi (boulders)
; Lowest 4 bits of EnIshi spawn variable is in t8
; Player's current strength level is in v0 - 0 = none, 1 = Goron Bracelet, 2 = Silver Gauntlets, 3 = Gold Gauntlets
; AT contains 1 (for silver boulder) when entering this function
; Set AT to 0 if we can lift the boulder
Player_CanLiftIshi:
; if actor variable == 1 it's a silver boulder, check for silver gauntlets
    beqz    t8, @@can_lift
    nop
@@check_silver_boulder:
    bne     t8, at, @@check_gold_boulder
    ori     at, r0, 0x03
    ; It's a silver boulder so check for str2
    ori     at, r0, 0x02
    blt     v0, at, @@cant_lift
    nop
    b       @@can_lift
    nop
@@check_gold_boulder:
    bne     t8, at, @@cant_lift
    nop
    ; It's a gold boulder so check for str3
    ori     at, r0, 0x03
    bge     v0, at, @@can_lift
    nop

@@cant_lift:
    jr      ra
    ori     at, r0, 1
@@can_lift:
    jr      ra
    or      at, r0, r0
