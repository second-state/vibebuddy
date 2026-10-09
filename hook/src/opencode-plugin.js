// VibeBuddy: tells the box what OpenCode is doing. Written by VibeBuddy; remove it from its Agents settings.
// Everything goes through vibebuddy-hook, which keeps only session ids, the working directory and its own
// verdict on the last reply: prompts and replies never leave this machine.
const HOOK = __HOOK__;

export const VibeBuddy = async ({ directory }) => {
  // The user message that started each session's current turn: its parts are the prompt, not the reply.
  const turns = new Map();
  // The latest text of the reply, for the hook to tell a finished task from a question.
  const replies = new Map();
  // Subagent sessions report to their parent session, not to the user.
  const children = new Set();

  const send = (event, sessionID, extra = {}) => {
    if (!sessionID || children.has(sessionID)) return;
    try {
      const hook = Bun.spawn([HOOK, "opencode"], { stdin: "pipe", stdout: "ignore", stderr: "ignore" });
      hook.stdin.write(JSON.stringify({ event, session_id: sessionID, turn_id: turns.get(sessionID), cwd: directory, ...extra }));
      hook.stdin.end();
    } catch {
      // VibeBuddy isn't installed any more, or can't run: OpenCode carries on regardless.
    }
  };

  return {
    "chat.message": async (input, output) => {
      turns.set(input.sessionID, output?.message?.id ?? input.messageID);
      replies.delete(input.sessionID);
      send("working", input.sessionID);
    },
    "tool.execute.after": async (input) => send("working", input.sessionID),
    event: async ({ event }) => {
      const properties = event.properties ?? {};
      switch (event.type) {
        case "session.created":
          if (properties.info?.parentID) children.add(properties.info.id ?? properties.sessionID);
          break;
        case "message.part.updated": {
          const part = properties.part;
          if (part?.type === "text" && !part.synthetic && part.messageID !== turns.get(part.sessionID)) {
            replies.set(part.sessionID, part.text);
          }
          break;
        }
        case "permission.asked":
        case "permission.updated":
        case "question.asked":
          send("needs_input", properties.sessionID);
          break;
        case "session.error":
          send("stopped", properties.sessionID);
          break;
        case "session.idle":
          send("done", properties.sessionID, { last_assistant_message: replies.get(properties.sessionID) });
          break;
        case "session.deleted":
          send("session_end", properties.sessionID);
          children.delete(properties.sessionID);
          break;
      }
    },
  };
};
