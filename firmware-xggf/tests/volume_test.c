#include "volume.h"
#include <assert.h>
int main(void) {
    unsigned level=20, seen=0;
    do { assert(volume_valid(level)); assert(level>=20 && level<=100); ++seen; level=volume_next(level); } while(level!=20 && seen<10);
    assert(seen==6); assert(volume_next(0)==20); assert(!volume_valid(0));
    assert(volume_next(20)==35 && volume_next(80)==100);
}
