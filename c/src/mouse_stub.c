#include "ghostos/mouse_stub.h"

ghostos_mouse_state ghostos_mouse_stub_state_read(void) {
    return (ghostos_mouse_state){0};
}
