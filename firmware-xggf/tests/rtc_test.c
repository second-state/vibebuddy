#include "rtc.h"
#include <assert.h>
int main(void) {
    xggf_clock c={0},decoded={0};uint8_t raw[7],control[]={0x20,0x20};
    assert(clock_sync(&c,20261103,23,45,0)); assert(rtc_encode(&c,raw));
    assert(raw[3]==2 && raw[1]==0x45 && raw[2]==0x23);
    assert(rtc_decode(raw,control,123,&decoded));assert(decoded.day==c.day && decoded.minute==c.minute);
    control[1]=0x30;assert(!rtc_decode(raw,control,0,&decoded));
    control[1]=0;assert(!rtc_decode(raw,control,0,&decoded));
    control[1]=0x20;control[0]=0;assert(!rtc_decode(raw,control,0,&decoded));
    control[0]=0x20;raw[1]=0x6a;assert(!rtc_decode(raw,control,0,&decoded));
    assert(rtc_encode(&c,raw));raw[5]=0x13;assert(!rtc_decode(raw,control,0,&decoded));
}
