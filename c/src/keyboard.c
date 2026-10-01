#include "ghostos/keyboard.h"

#include <stdatomic.h>

enum { ENABLE_FIRST_PORT=0xae, ENABLE_SECOND_PORT=0xa8, WRITE_AUXILIARY=0xd4,
    ENABLE_SCANNING=0xf4, SET_DEFAULTS=0xf6, OUTPUT_FULL=1, INPUT_FULL=1u<<1, AUXILIARY_DATA=1u<<5 };

void ghostos_keyboard_init(ghostos_keyboard *k) { *k = (ghostos_keyboard){0}; }

void ghostos_keyboard_create(ghostos_keyboard *k, const ghostos_keyboard_io *io) {
    ghostos_keyboard_init(k);
    ghostos_keyboard_initialize_controller(io);
}

static bool wait_for_input_buffer(const ghostos_keyboard_io *io) {
    if (!io || !io->read_port) return false;
    for (size_t i=0;i<100000;i++) {
        if (!(io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT)&INPUT_FULL)) return true;
        if (io->spin) io->spin(io->context);
    }
    return false;
}

static bool send_auxiliary(const ghostos_keyboard_io *io, uint8_t command) {
    if (!io || !io->write_port || !io->read_port || !wait_for_input_buffer(io)) return false;
    io->write_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT,WRITE_AUXILIARY);
    if (!wait_for_input_buffer(io)) return false;
    io->write_port(io->context,GHOSTOS_PS2_KEYBOARD_DATA_PORT,command);
    for (size_t i=0;i<100000;i++) {
        uint8_t status=io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT);
        if ((status&OUTPUT_FULL)&&(status&AUXILIARY_DATA))
            return io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_DATA_PORT)==0xfa;
        if (io->spin) io->spin(io->context);
    }
    return false;
}

void ghostos_keyboard_initialize_controller(const ghostos_keyboard_io *io) {
    if (!io || !io->read_port || !io->write_port) return;
    for (size_t i=0;i<32;i++) {
        if (!(io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT)&OUTPUT_FULL)) break;
        (void)io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_DATA_PORT);
    }
    if (wait_for_input_buffer(io)) io->write_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT,ENABLE_FIRST_PORT);
    if (wait_for_input_buffer(io)) io->write_port(io->context,GHOSTOS_PS2_KEYBOARD_DATA_PORT,ENABLE_SCANNING);
    if (wait_for_input_buffer(io)) io->write_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT,ENABLE_SECOND_PORT);
    (void)send_auxiliary(io,SET_DEFAULTS);
    (void)send_auxiliary(io,ENABLE_SCANNING);
}

static bool shifted(const ghostos_keyboard *k) { return k->left_shift || k->right_shift; }

static bool letter(uint8_t code, uint8_t *out) {
    static const struct { uint8_t code, value; } letters[] = {
        {0x10,'q'},{0x11,'w'},{0x12,'e'},{0x13,'r'},{0x14,'t'},{0x15,'y'},{0x16,'u'},{0x17,'i'},{0x18,'o'},{0x19,'p'},
        {0x1e,'a'},{0x1f,'s'},{0x20,'d'},{0x21,'f'},{0x22,'g'},{0x23,'h'},{0x24,'j'},{0x25,'k'},{0x26,'l'},
        {0x2c,'z'},{0x2d,'x'},{0x2e,'c'},{0x2f,'v'},{0x30,'b'},{0x31,'n'},{0x32,'m'}
    };
    for (size_t i=0;i<sizeof(letters)/sizeof(letters[0]);i++) if (letters[i].code==code) { *out=letters[i].value; return true; }
    return false;
}

static bool symbol(uint8_t code, uint8_t *plain, uint8_t *upper) {
    static const struct { uint8_t code, plain, upper; } symbols[] = {
        {0x02,'1','!'},{0x03,'2','@'},{0x04,'3','#'},{0x05,'4','$'},{0x06,'5','%'},{0x07,'6','^'},
        {0x08,'7','&'},{0x09,'8','*'},{0x0a,'9','('},{0x0b,'0',')'},{0x0c,'-','_'},{0x0d,'=','+'},
        {0x1a,'[','{'},{0x1b,']','}'},{0x27,';',':'},{0x28,'\'','"'},{0x29,'`','~'},
        {0x2b,'\\','|'},{0x33,',','<'},{0x34,'.','>'},{0x35,'/','?'}
    };
    for (size_t i=0;i<sizeof(symbols)/sizeof(symbols[0]);i++) if (symbols[i].code==code) {
        *plain=symbols[i].plain; *upper=symbols[i].upper; return true;
    }
    return false;
}

static bool take_pending(ghostos_keyboard *k, uint8_t *out) {
    if (!k->pending_count) return false;
    *out=k->pending[k->pending_start];
    k->pending_start=(uint8_t)((k->pending_start+1)%sizeof(k->pending));
    --k->pending_count;
    return true;
}

static bool queue_sequence(ghostos_keyboard *k, const uint8_t *sequence, size_t length, uint8_t *out) {
    for (size_t i=0;i<length;i++) {
        if (k->pending_count==sizeof(k->pending)) break;
        size_t index=(k->pending_start+k->pending_count)%sizeof(k->pending);
        k->pending[index]=sequence[i];
        ++k->pending_count;
    }
    return take_pending(k,out);
}

static bool extended_key(ghostos_keyboard *k, uint8_t code, bool released, uint8_t *out) {
    static const uint8_t home[]="\x1b[H", shift_home[]="\x1b[1;2H";
    static const uint8_t up[]="\x1b[A", shift_up[]="\x1b[1;2A";
    static const uint8_t left[]="\x1b[D", shift_left[]="\x1b[1;2D";
    static const uint8_t right[]="\x1b[C", shift_right[]="\x1b[1;2C";
    static const uint8_t end[]="\x1b[F", down[]="\x1b[B", shift_down[]="\x1b[1;2B", del[]="\x1b[3~";
    if (code==0x1d) { k->control=!released; return false; }
    if (released) return false;
    const uint8_t *seq=0; size_t len=0;
    switch(code) {
        case 0x1c: *out='\r'; return true;
        case 0x47: seq=shifted(k)?shift_home:home; len=shifted(k)?sizeof(shift_home)-1:sizeof(home)-1; break;
        case 0x48: seq=shifted(k)?shift_up:up; len=shifted(k)?sizeof(shift_up)-1:sizeof(up)-1; break;
        case 0x4b: seq=shifted(k)?shift_left:left; len=shifted(k)?sizeof(shift_left)-1:sizeof(left)-1; break;
        case 0x4d: seq=shifted(k)?shift_right:right; len=shifted(k)?sizeof(shift_right)-1:sizeof(right)-1; break;
        case 0x4f: seq=end; len=sizeof(end)-1; break;
        case 0x50: seq=shifted(k)?shift_down:down; len=shifted(k)?sizeof(shift_down)-1:sizeof(down)-1; break;
        case 0x53: seq=del; len=sizeof(del)-1; break;
        default: return false;
    }
    return queue_sequence(k,seq,len,out);
}

static bool decode_character(const ghostos_keyboard *k, uint8_t code, uint8_t *out) {
    uint8_t value;
    if (letter(code,&value)) {
        if (k->control) { *out=value&0x1f; return true; }
        bool uppercase=shifted(k)^k->caps_lock;
        *out=uppercase?(uint8_t)(value-'a'+'A'):value;
        return true;
    }
    uint8_t plain, upper;
    if (!symbol(code,&plain,&upper)) return false;
    *out=shifted(k)?upper:plain;
    return true;
}

bool ghostos_keyboard_read_byte(ghostos_keyboard *k, const ghostos_keyboard_io *io, uint8_t *out) {
    if (!k || !io || !io->read_port || !out) return false;
    for (;;) {
        if (take_pending(k,out)) return true;
        uint8_t status=io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_STATUS_PORT);
        if (!(status&OUTPUT_FULL)) return false;
        uint8_t scan=io->read_port(io->context,GHOSTOS_PS2_KEYBOARD_DATA_PORT);
        if (status&AUXILIARY_DATA) { if (io->mouse_byte) io->mouse_byte(io->context,scan); continue; }
        if (scan==0xe0) { k->extended=true; continue; }
        if (scan==0xfa||scan==0xfe) continue;
        bool released=(scan&0x80)!=0; uint8_t code=scan&0x7f;
        if (k->extended) { k->extended=false; if (extended_key(k,code,released,out)) return true; continue; }
        switch(code) {
            case 0x1d: k->control=!released; break;
            case 0x2a: k->left_shift=!released; break;
            case 0x36: k->right_shift=!released; break;
            case 0x3a: if (!released) k->caps_lock=!k->caps_lock; break;
            default:
                if (!released) {
                    if (code==0x01) { *out=3; return true; }
                    if (code==0x0e) { *out=8; return true; }
                    if (code==0x0f) { *out='\t'; return true; }
                    if (code==0x1c) { *out='\r'; return true; }
                    if (code==0x39) { *out=' '; return true; }
                    if (decode_character(k,code,out)) return true;
                }
                break;
        }
    }
}

static ghostos_keyboard boot_keyboard;
static atomic_flag boot_lock=ATOMIC_FLAG_INIT;
static atomic_bool boot_ready=ATOMIC_VAR_INIT(false);
bool ghostos_keyboard_read_boot_byte(const ghostos_keyboard_io *io, uint8_t *byte) {
    while (atomic_flag_test_and_set_explicit(&boot_lock,memory_order_acquire)) atomic_signal_fence(memory_order_seq_cst);
    if (!atomic_exchange_explicit(&boot_ready,true,memory_order_acq_rel)) ghostos_keyboard_init(&boot_keyboard);
    bool result=ghostos_keyboard_read_byte(&boot_keyboard,io,byte);
    atomic_flag_clear_explicit(&boot_lock,memory_order_release);
    return result;
}

#if defined(__x86_64__)
static uint8_t x86_read_port(void *context, uint16_t port) {
    (void)context;
    uint8_t value;
    __asm__ volatile ("inb %1, %0" : "=a"(value) : "Nd"(port));
    return value;
}
static void x86_write_port(void *context, uint16_t port, uint8_t value) {
    (void)context;
    __asm__ volatile ("outb %0, %1" : : "a"(value), "Nd"(port));
}
static void x86_spin(void *context) { (void)context; __asm__ volatile ("pause"); }
bool ghostos_keyboard_x86_io(ghostos_keyboard_io *io, void *context,
    void (*mouse_byte)(void *context, uint8_t byte)) {
    if (!io) return false;
    *io = (ghostos_keyboard_io){context, x86_read_port, x86_write_port, x86_spin, mouse_byte};
    return true;
}
#else
bool ghostos_keyboard_x86_io(ghostos_keyboard_io *io, void *context,
    void (*mouse_byte)(void *context, uint8_t byte)) {
    (void)io; (void)context; (void)mouse_byte;
    return false;
}
#endif
