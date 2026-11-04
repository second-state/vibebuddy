#include "clock.h"
#include <assert.h>
int main(void) {
    xggf_clock c={0};
    assert(!clock_sync(&c,20260229,12,0,0)); assert(!c.valid);
    assert(!clock_sync(&c,20260431,12,0,0));
    assert(!clock_sync(&c,20260001,12,0,0));
    assert(!clock_sync(&c,20261301,12,0,0));
    assert(!clock_sync(&c,20261103,24,0,0));
    assert(!clock_sync(&c,20261103,0,60,0));
    assert(clock_sync(&c,20261231,23,59,0)); clock_tick(&c,60000);
    assert(c.day==20270101 && c.minute==0);
    assert(clock_sync(&c,20280228,23,59,UINT32_MAX-1000));
    clock_tick(&c,58999); assert(c.day==20280229 && c.minute==0);
    clock_tick(&c,118999); assert(c.minute==1);
}
