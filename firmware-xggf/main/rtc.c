#include "rtc.h"
static uint8_t bcd(unsigned n) { return (uint8_t)((n/10)*16+n%10); }
bool rtc_decode(const uint8_t raw[7],const uint8_t control[2],uint32_t now,xggf_clock *clock)
{
    unsigned v[7];
    /* Require 24-hour mode, a running oscillator and no power-on reset. */
    if(!(control[0]&0x20) || (control[1]&0x30)!=0x20) return false;
    for(unsigned i=0;i<7;++i) {
        if((raw[i]&15)>9 || (raw[i]>>4)>9) return false;
        v[i]=(raw[i]>>4)*10+(raw[i]&15);
    }
    if(v[0]>59 || v[3]>6) return false;
    return clock_sync(clock,(2000+v[6])*10000+v[5]*100+v[4],v[2],v[1],now);
}
bool rtc_encode(const xggf_clock *clock,uint8_t raw[7])
{
    xggf_clock checked={0};
    if(!clock->valid || !clock_sync(&checked,clock->day,clock->minute/60,clock->minute%60,0)) return false;
    unsigned year=clock->day/10000,month=clock->day/100%100,day=clock->day%100;
    /* Gregorian weekday, Sunday zero. */
    static const unsigned offsets[]={0,3,2,5,0,3,5,1,4,6,2,4};
    unsigned y=year-(month<3);
    unsigned weekday=(y+y/4-y/100+y/400+offsets[month-1]+day)%7;
    raw[0]=0;raw[1]=bcd(clock->minute%60);raw[2]=bcd(clock->minute/60);
    raw[3]=bcd(weekday);raw[4]=bcd(day);raw[5]=bcd(month);raw[6]=bcd(year-2000);
    return true;
}
