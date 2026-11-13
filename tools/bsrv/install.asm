; One-shot installer, uploaded through DEBUG. Code D800, padded COM E000.
; No overwrite: checks B:BSRV.COM in user 0 before creating it.
; Status is sent over SIO-B as B8I:<code> (K=verified, E=exists).
        org 0d800h
start:  di
        ld sp,0dff0h
        ld c,32
        ld e,0ffh
        call 5
        ld (old_user),a
        ld c,32
        ld e,0
        call 5
        ld de,fcb
        ld c,15
        call 5
        cp 0ffh
        jr z,create
        ld de,fcb
        ld c,16
        call 5
        ld a,'E'
        jp finish
create: ld de,fcb
        ld c,22
        call 5
        cp 0ffh
        ld a,'C'
        jp z,finish
        ld hl,0e000h
        ld (pointer),hl
        ld a,5
        ld (remaining),a
write:  ld de,(pointer)
        ld c,26
        call 5
        ld de,fcb
        ld c,21
        call 5
        or a
        ld a,'W'
        jp nz,finish
        ld hl,(pointer)
        ld de,128
        add hl,de
        ld (pointer),hl
        ld hl,remaining
        dec (hl)
        jr nz,write
        ld de,fcb
        ld c,16
        call 5
        cp 0ffh
        ld a,'C'
        jp z,finish
; Reopen and compare every byte of all five records with uploaded data.
        ld hl,fcb+12
        ld b,24
        xor a
clear_fcb: ld (hl),a
        inc hl
        djnz clear_fcb
        ld de,fcb
        ld c,15
        call 5
        cp 0ffh
        ld a,'O'
        jp z,finish
        ld hl,0e000h
        ld (pointer),hl
        ld a,5
        ld (remaining),a
verify: ld de,0e800h
        ld c,26
        call 5
        ld de,fcb
        ld c,20
        call 5
        or a
        ld a,'R'
        jp nz,finish
        ld hl,(pointer)
        ld de,0e800h
        ld b,128
compare: ld a,(de)
        cp (hl)
        ld a,'V'
        jp nz,finish
        inc hl
        inc de
        djnz compare
        ld (pointer),hl
        ld hl,remaining
        dec (hl)
        jr nz,verify
        ld de,fcb
        ld c,16
        call 5
        cp 0ffh
        ld a,'C'
        jr z,finish
        ld a,'K'
finish: ld (status),a
        ld a,(old_user)
        ld e,a
        ld c,32
        call 5
        ld hl,message
        ld b,4
send_prefix: ld a,(hl)
        call tx
        inc hl
        djnz send_prefix
        ld a,(status)
        call tx
        ld a,13
        call tx
        ld a,10
        call tx
        jp 0f203h
; Wait for SIO-B ALL SENT, preserve AF/BC.
tx:     push af
        push bc
        ld c,a
wait:   ld a,1
        out (083h),a
        in a,(083h)
        and 1
        jr z,wait
        ld a,c
        out (081h),a
        pop bc
        pop af
        ret
message: db 'B8I:'
status: db 0
old_user: db 0
remaining: db 0
pointer: dw 0
fcb:    db 2,'BSRV    COM'
        ds 24
