#pragma once

#include <stdbool.h>
#include <stdint.h>

/// 休闲模式的导演：管无聊度、抽剧目、决定什么时候转暗和关背光。
/// 不读时钟、不碰硬件，所有入口都由调用方传入毫秒计数，允许回绕。

/// 空闲多久算无聊、多久算困倦、夜里困倦多久后关背光。
#define AGENT_LEISURE_BORED_AFTER_MS (5u * 60u * 1000u)
#define AGENT_LEISURE_SLEEPY_AFTER_MS (30u * 60u * 1000u)
#define AGENT_LEISURE_LIGHTS_OUT_AFTER_MS (90u * 60u * 1000u)
/// 剧目动画的帧长：8 fps。
#define AGENT_LEISURE_FRAME_MS 125u

typedef enum {
  /// 待命：现有的呼吸、眨眼、轮播战绩。
  AGENT_LEISURE_ALERT,
  /// 无聊：隔一会儿演一段小剧目。
  AGENT_LEISURE_BORED,
  /// 困倦：以睡觉为主，画面转暗。
  AGENT_LEISURE_SLEEPY,
} agent_leisure_tier_t;

typedef enum {
  /// 剧目之间的普通空闲。
  AGENT_SKIT_NONE,
  AGENT_SKIT_PATROL,
  AGENT_SKIT_BALL,
  AGENT_SKIT_READ,
  AGENT_SKIT_STARS,
  AGENT_SKIT_HIDE,
  AGENT_SKIT_STARTLE,
  AGENT_SKIT_DREAM,
  /// 困倦期的底色：睡觉。
  AGENT_SKIT_SLEEP,
  AGENT_SKIT_COUNT,
} agent_skit_t;

typedef struct {
  agent_leisure_tier_t tier;
  agent_skit_t skit;
  /// 当前剧目已经演了几帧。
  uint32_t skit_frame;
  /// 困倦：画面转暗。
  bool dim;
  /// 夜里睡久了：关背光。
  bool lights_out;
} agent_leisure_view_t;

void agent_leisure_init(uint32_t seed, uint32_t now_ms);
/// 任何活动都把无聊度清零：Agent 有动静、按键、番茄钟在走。
void agent_leisure_note_activity(uint32_t now_ms);
/// K1 长按：现在就去玩。
void agent_leisure_force_bored(uint32_t now_ms);
/// 本地小时数，来自 Mac 端心跳；-1 表示不知道。
void agent_leisure_set_hour(int hour);
/// 当日完成的专注或任务数，决定它是累了还是无聊。
void agent_leisure_set_done_count(unsigned done);
/// 推进导演。档位或剧目变了返回 true。
bool agent_leisure_tick(uint32_t now_ms);
void agent_leisure_view(uint32_t now_ms, agent_leisure_view_t *view);
/// 测试与预览用：立刻开演某个剧目。
void agent_leisure_start_skit(agent_skit_t skit, uint32_t now_ms);

const char *agent_leisure_tier_name(agent_leisure_tier_t tier);
const char *agent_leisure_skit_name(agent_skit_t skit);
