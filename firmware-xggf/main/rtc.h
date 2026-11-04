#ifndef XGGF_RTC_H
#define XGGF_RTC_H
#include "clock.h"
/* RX-8025-compatible wire format recovered from the stock firmware. */
bool rtc_decode(const uint8_t raw[7],const uint8_t control[2],uint32_t now,xggf_clock *clock);
bool rtc_encode(const xggf_clock *clock,uint8_t raw[7]);
#endif
