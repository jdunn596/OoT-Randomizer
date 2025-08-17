; Hacks in EnGs (Gossip Stone)
.headersize(0x80B6C070 - 0xEE7790)
; ==============================================================
; Gossip Stone Shuffle
; ==============================================================

; Hook gossip stone action function that is checking for a song
.org 0x80B6C1CC
; Replaces: Entire Function
    j       En_Gs_Update_Hack
    nop
