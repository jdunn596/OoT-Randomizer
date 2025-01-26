; Hacks in EffectSsKakera

.headersize(0x80B34AB0 - 0xEB01E0)

; Hack in EffectSsKakera_Draw to use expanded color table
.org 0x80b34e34 ; (reloc 0x384)
; Replaces:
;   lui     t7, 0x80B3
    lui     t7, hi(sEffectSsKakera_Colors)
.org 0x80b34e48 ; (reloc 0x398)
; Replaces:
;   addiu   t7, t7, 0x5918
    addiu   t7, t7, lo(sEffectSsKakera_Colors)
.org 0x80b34ee4 ; (reloc 0x434)
; Replaces:
;   lui     t9, 0x80B3
    lui     t9, hi(sEffectSsKakera_Colors)
.org 0x80b34ef8 ; (reloc 0x448)
; Replaces:
;   addiu   t9, t9, 0x5918
    addiu   t9, t9, lo(sEffectSsKakera_Colors)

; Patch relocs
.org 0x80b35a40 ; 0x384
    nop
.org 0x80b35a44 ; 0x398
    nop
.org 0x80b35a48 ; 0x434
    nop
.org 0x80b35a4c ; 0x448
    nop