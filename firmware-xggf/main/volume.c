#include "volume.h"
static const unsigned steps[]={20,35,50,65,80,100};
bool volume_valid(unsigned level)
{
    for(unsigned i=0;i<6;++i) if(level==steps[i]) return true;
    return false;
}
unsigned volume_next(unsigned level)
{
    for(unsigned i=0;i<6;++i) if(level==steps[i]) return steps[(i+1)%6];
    return 20;
}
