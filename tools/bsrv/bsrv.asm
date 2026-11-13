; Bosman UNIMOR ZJ disk server for the SAJ D/E/F client.
; z80asm -o tools/bsrv/BSRV.COM tools/bsrv/bsrv.asm
; Standard CP/M .COM entry. Relocate above the ROM window: ZJ BIOS HOME
; enables ROM without restoring the caller's mapping.
        org 0100h
        ld de,banner
        ld c,9
        call 5
        di
        ld hl,image
        ld de,0d000h
        ld bc,image_end-0d000h
        ldir
        jp 0d000h
banner: db 13,10,'BSRV 1.0 - Bosman ZJ / SAJ disk server',13,10
        db 'A/B/C shared as D/E/F. Client DIR G: exits.',13,10,'$'
image:
; Labels below are relocated by defining their logical addresses explicitly.
        org 0d000h
start:  ld sp,0d800h
        ld a,13
        call line
        xor a
        ld (ready),a
slow:   call rx
        cp 05dh
        jr z,hello
        cp 056h
        jr z,addr_lo
        cp 058h
        jr z,addr_hi
        cp 05bh
        jr z,read_mem
        cp 060h
        jr z,enter_disk
        jr slow
hello:  xor a
        call tx
        ld a,088h
        call tx
        jr slow
addr_lo: call rx
        ld (address),a
        jr slow
addr_hi: call rx
        ld (address+1),a
        jr slow
read_mem: ld hl,(address)
        ld a,(hl)
        inc hl
        ld (address),hl
        call tx
        jr slow
enter_disk: call rx
        ld a,6
        call line
        ld bc,0e800h
        call 0f224h
loop:   call rx
        cp 0f0h
        jp z,quit
        cp 0f1h
        jr z,header
        cp 0f2h
        jp z,read_sector
        cp 0f3h
        jp z,write_sector
        jr loop
header: ld b,0f1h
        ld hl,hdr
        ld e,5
header_rx: call rx
        ld (hl),a
        inc hl
        add a,b
        ld b,a
        dec e
        jr nz,header_rx
        ld a,b
        or a
        jr nz,header_reply
        ld a,(hdr+3)
        cp 3
        jr nc,bad_drive
        xor a
        ld (ready),a
        ld a,(hdr)
        or a
        call nz,0f218h
        ld a,(hdr+1)
        ld c,a
        ld b,0
        call 0f21eh
        ld a,(hdr+2)
        ld c,a
        ld b,0
        call 0f221h
        ld a,(hdr+3)
        ld c,a
        ld b,0
        ld e,0
        call 0f21bh
        ld a,h
        or l
        jr z,bad_drive
        ld a,1
        ld (ready),a
        xor a
header_reply: call tx
        jp loop
bad_drive: ld a,05ah
        jr header_reply
read_sector:
        call rx
        add a,0f2h
        call tx
        or a
        jp nz,loop
        ld a,(ready)
        or a
        jp z,loop
        call 0f227h
        ld hl,0e800h
        ld b,0
        ld e,128
read_send: ld a,(hl)
        ld c,a
        add a,b
        ld b,a
        ld a,c
        call tx
        inc hl
        dec e
        jr nz,read_send
        xor a
        sub b
        call tx
        jp loop
write_sector:
        call rx
        ld (write_type),a
        add a,0f3h
        ld b,a
        ld hl,0e800h
        ld e,128
write_rx: call rx
        ld (hl),a
        inc hl
        add a,b
        ld b,a
        dec e
        jr nz,write_rx
        call rx
        add a,b
        jp nz,header_reply
        ld a,(ready)
        or a
        jp z,bad_drive
        ld a,(write_type)
        cp 3
        jp nc,bad_drive
        ld c,a
        call 0f22ah
        jp header_reply
quit:   ld a,13
        call line
        call rx
        jp 0f203h
; Direct SIO-B: preserve registers and wait for ALL SENT, like the ROM.
tx:     push af
        push bc
        ld c,a
tx_wait: ld a,1
        out (083h),a
        in a,(083h)
        and 1
        jr z,tx_wait
        ld a,c
        out (081h),a
        pop bc
        pop af
        ret
rx:     in a,(083h)
        and 1
        jr z,rx
        in a,(081h)
        ret
; A = 8253 counter 2. Keep receive FIFO across the speed switch, as SAJ
; does: an in-memory/TCP peer may already have queued the next header.
; Set 8N2 x16, RTS/DTR asserted.
line:   push af
        ld a,4
        out (083h),a
        ld a,04ch
        out (083h),a
        ld a,3
        out (083h),a
        ld a,0e1h
        out (083h),a
        ld a,5
        out (083h),a
        ld a,0eah
        out (083h),a
        ld a,0b6h
        out (087h),a
        pop af
        out (086h),a
        xor a
        out (086h),a
        in a,(081h)
        ld bc,400
settle: dec bc
        ld a,b
        or c
        jr nz,settle
        in a,(081h)
        ret
address: dw 0
hdr:    ds 5
ready:  db 0
write_type: db 0
image_end:
