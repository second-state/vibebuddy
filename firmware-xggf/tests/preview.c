/* Renders every status label scenario to PBM files, native and 8x enlarged:
   preview OUTDIR. View the files with any image viewer (they are 1-bit bitmaps). */
#include "panel.h"
#include "status.h"
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <sys/types.h>

static void write_pbm(const char *dir, const char *name, unsigned scale, const uint8_t *frame)
{
    char path[512];
    snprintf(path, sizeof(path), "%s/%s-%ux.pbm", dir, name, scale);
    FILE *out = fopen(path, "w");
    if (!out) { fprintf(stderr, "preview: %s: %s\n", path, strerror(errno)); exit(1); }
    fprintf(out, "P1\n%u %u\n", PANEL_WIDTH * scale, PANEL_HEIGHT * scale);
    for (unsigned y = 0; y < PANEL_HEIGHT * scale; ++y) {
        for (unsigned x = 0; x < PANEL_WIDTH * scale; ++x) {
            const uint8_t *rgb = frame + ((y / scale) * PANEL_WIDTH + x / scale) * 3;
            fprintf(out, "%u%c", rgb[0] || rgb[1] || rgb[2] ? 1u : 0u, x + 1 == PANEL_WIDTH * scale ? '\n' : ' ');
        }
    }
    fclose(out);
    printf("%s\n", path);
}

static void write_ppm(const char *dir, const char *name, unsigned scale, const uint8_t *frame)
{
    char path[512];
    snprintf(path,sizeof(path),"%s/%s-%ux.ppm",dir,name,scale);
    FILE *out=fopen(path,"wb");
    if(!out) { perror(path); exit(1); }
    fprintf(out,"P6\n%u %u\n255\n",PANEL_WIDTH*scale,PANEL_HEIGHT*scale);
    for(unsigned y=0;y<PANEL_HEIGHT*scale;++y) for(unsigned x=0;x<PANEL_WIDTH*scale;++x) {
        const uint8_t *rgb=frame+((y/scale)*PANEL_WIDTH+x/scale)*3;
        for(unsigned c=0;c<3;++c) fputc(rgb[c]*31,out);
    }
    fclose(out);
}

static void emit(const char *dir, const char *name, const status_model *m, uint32_t now)
{
    static uint8_t frame[PANEL_RGB_BYTES];
    status_render(m, now, frame);
    write_pbm(dir, name, 1, frame);
    write_pbm(dir, name, 8, frame);
    write_ppm(dir,name,1,frame);
    write_ppm(dir,name,8,frame);
}

static void agent_state(const char *dir, const char *name, const char *event)
{
    status_model m;
    status_init(&m);
    status_event(&m, event, 100);
    emit(dir, name, &m, 100);
}

int main(int argc, char **argv)
{
    if (argc != 2) { fprintf(stderr, "usage: preview OUTDIR\n"); return 1; }
    if (mkdir(argv[1], 0777) != 0 && errno != EEXIST) {
        fprintf(stderr, "preview: %s: %s\n", argv[1], strerror(errno));
        return 1;
    }

    status_model m;
    status_init(&m);
    emit(argv[1], "offline", &m, 0);

    agent_state(argv[1], "ready", "device.heartbeat");
    agent_state(argv[1], "working", "task.start");
    agent_state(argv[1], "input", "agent.input_required");
    agent_state(argv[1], "done", "task.done");
    agent_state(argv[1], "failed", "task.error");

    status_init(&m);
    status_event(&m, "device.heartbeat", 0);
    status_toggle_mute(&m, 0);
    emit(argv[1], "mute-on", &m, 100);
    status_toggle_mute(&m, 100);
    emit(argv[1], "mute-off", &m, 200);

    /* Pressed-key dots: a temporary 2x2 purple dot per held key, rows 0-1, over the current view. */
    status_init(&m);
    bool pressed[3] = {true, false, false}, long_fired[3] = {false, false, false};
    status_buttons(&m, pressed, long_fired);
    emit(argv[1], "k0-down", &m, 0);
    pressed[0] = true; pressed[1] = true; pressed[2] = true;
    status_buttons(&m, pressed, long_fired);
    emit(argv[1], "keys-all-down", &m, 0);

    status_init(&m);
    status_set_recording(&m, true);
    emit(argv[1], "rec", &m, 0);

    status_init(&m);
    status_sensors(&m, true, 12, 34, false, 0, 0, 0);
    m.view = STATUS_VIEW_CLOCK;
    emit(argv[1], "clock", &m, 0);

    status_init(&m);
    status_sensors(&m, false, 0, 0, true, -120, 580, 0);
    m.view = STATUS_VIEW_ENVIRONMENT;
    emit(argv[1], "environment", &m, 0);

    status_init(&m);
    status_sensors(&m, false, 0, 0, false, 0, 0, 0);
    m.view = STATUS_VIEW_CLOCK;
    emit(argv[1], "no-rtc", &m, 0);

    status_init(&m);
    status_sensors(&m, false, 0, 0, false, 0, 0, 0);
    m.view = STATUS_VIEW_ENVIRONMENT;
    emit(argv[1], "no-env", &m, 0);

    /* Connection indicator, rightmost column: rows 0-3 network, rows 4-7 host link. */
    status_init(&m);
    emit(argv[1], "link-unset", &m, 0);

    status_init(&m);
    status_event(&m, "device.heartbeat", 0);
    emit(argv[1], "link-host-ok", &m, 100);

    status_init(&m);
    status_network(&m, true, true);
    status_event(&m, "device.heartbeat", 0);
    emit(argv[1], "link-ok", &m, 100);

    status_init(&m);
    status_network(&m, true, false);
    status_event(&m, "device.heartbeat", 0);
    emit(argv[1], "link-net-error", &m, 100);

    status_init(&m);
    status_network(&m, true, true);
    status_event(&m, "device.heartbeat", 0);
    status_tick(&m, 15000);
    emit(argv[1], "link-host-error", &m, 15000);

    status_init(&m);
    button_actions_handle(&m.button_actions, BUTTON_K1_LONG, 0);
    emit(argv[1], "menu-mic", &m, 0);
    button_actions_handle(&m.button_actions, BUTTON_K1_SHORT, 1);
    emit(argv[1], "menu-link-row", &m, 1);
    link_status_info info = { .wifi_state = LINK_WIFI_JOINED, .ssid = "Cafe WiFi", .ip = "192.168.1.20", .paired_count = 2 };
    status_link_update(&m, &info);
    status_event(&m, "device.heartbeat", 2);
    status_transport_update(&m, true);
    button_actions_handle(&m.button_actions, BUTTON_K0_SHORT, 2);
    emit(argv[1], "menu-link-usb", &m, 2);
    button_actions_handle(&m.button_actions, BUTTON_K1_SHORT, 3);
    emit(argv[1], "menu-wifi", &m, 3);
    button_actions_handle(&m.button_actions, BUTTON_K1_SHORT, 4);
    emit(argv[1], "menu-ip", &m, 4);
    button_actions_handle(&m.button_actions, BUTTON_K1_SHORT, 5);
    emit(argv[1], "menu-pairs", &m, 5);
    button_actions_handle(&m.button_actions, BUTTON_K2_SHORT, 6);
    button_actions_handle(&m.button_actions, BUTTON_K1_SHORT, 7);
    m.brightness = 2;
    emit(argv[1], "menu-brightness", &m, 7);

    status_init(&m);
    pressed[0] = pressed[2] = false; long_fired[0] = long_fired[2] = false;
    pressed[1] = true; long_fired[1] = true;
    status_buttons(&m, pressed, long_fired);
    emit(argv[1], "k1-long", &m, 1000);

    status_init(&m); status_sensors(&m,true,12,34,false,0,0,0);
    emit(argv[1],"offline-clock",&m,0);
    const char *names[]={"pomodoro-idle","focus-running","focus-paused","break-ready","break-running","alarm"};
    for(unsigned i=0;i<6;++i) {
        status_init(&m); m.view=STATUS_VIEW_POMODORO;
        m.pomodoro=(pomodoro_view){.phase=i>=3,.run=i==1||i==4?1:i==2?2:0,.total_ms=i>=3?300000:1500000,.remaining_ms=i>=3?300000:1500000,.completed=3};
        if(i==1||i==2) m.pomodoro.remaining_ms=750000;
        m.pomodoro_alarm=i==5; emit(argv[1],names[i],&m,0);
        if(i==2) emit(argv[1],"focus-paused-hidden",&m,500);
        if(i==5) {emit(argv[1],"alarm-hidden",&m,500);emit(argv[1],"alarm-pulse",&m,2500);}
    }
    for(unsigned i=0;i<4;++i) {
        status_init(&m); m.state=STATUS_INPUT; m.linked=true; m.view=(status_view)i;
        m.pomodoro=(pomodoro_view){.total_ms=1500000,.remaining_ms=1500000};
        char name[32]; snprintf(name,sizeof(name),"alert-page-%u",i); emit(argv[1],name,&m,0);
        m.button_actions.recording=true; snprintf(name,sizeof(name),"record-alert-%u",i); emit(argv[1],name,&m,0);
    }
    return 0;
}
