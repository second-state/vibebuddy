#include "clock.h"
static unsigned month_days(unsigned year, unsigned month)
{
    static const unsigned days[]={31,28,31,30,31,30,31,31,30,31,30,31};
    return days[month-1]+(month==2 && year%4==0 && (year%100!=0 || year%400==0));
}
bool clock_sync(xggf_clock *c,unsigned day,unsigned hour,unsigned minute,uint32_t now)
{
    unsigned y=day/10000,m=day/100%100,d=day%100;
    if(y<2000 || y>2099 || m<1 || m>12 || d<1 || d>month_days(y,m) || hour>23 || minute>59) return false;
    *c=(xggf_clock){true,day,hour*60+minute,now}; return true;
}
void clock_tick(xggf_clock *c,uint32_t now)
{
    if(!c->valid) return;
    unsigned elapsed=(uint32_t)(now-c->at_ms)/60000;
    c->at_ms+=elapsed*60000; c->minute+=elapsed;
    while(c->minute>=1440) {
        c->minute-=1440;
        unsigned y=c->day/10000,m=c->day/100%100,d=c->day%100+1;
        if(d>month_days(y,m)) {d=1;if(++m>12) {m=1;++y;}}
        c->day=y*10000+m*100+d;
    }
}
