// Vibe Buddy: tells the box what Pi is doing. Written by Vibe Buddy; remove it from its Agents settings.
// Everything goes through vibebuddy-hook, which keeps only session ids, the working directory and its own
// verdict on the last reply: prompts and replies never leave this machine.
import { spawn } from "node:child_process";

const HOOK = __HOOK__;

export default function (pi) {
  // Each agent run is one turn; its id only has to differ from the session's previous runs.
  let turn;
  let running = false;
  // The latest assistant message decides the outcome, so a retried error is replaced by its successful retry.
  let reply;
  let failed = false;

  // One hook at a time, in order: started together, a run's `done` can reach the box after the `session_end`
  // that follows it, and leave a card for a session that is gone.
  let queue = Promise.resolve();
  const send = (event, ctx, extra = {}) => {
    const sessionID = ctx.sessionManager.getSessionId();
    if (!sessionID) return queue;
    const payload = JSON.stringify({ event, session_id: sessionID, turn_id: turn, cwd: ctx.cwd, ...extra });
    queue = queue.then(
      () =>
        new Promise((resolve) => {
          try {
            const hook = spawn(HOOK, ["pi"], { stdio: ["pipe", "ignore", "ignore"] });
            hook.on("close", resolve);
            hook.on("error", resolve);
            hook.stdin.on("error", () => {});
            hook.stdin.end(payload);
          } catch {
            // Vibe Buddy isn't installed any more, or can't run: Pi carries on regardless.
            resolve();
          }
        }),
    );
    return queue;
  };

  pi.on("agent_start", (_event, ctx) => {
    turn = String(Date.now());
    running = true;
    reply = undefined;
    failed = false;
    send("working", ctx);
  });
  pi.on("tool_execution_end", (_event, ctx) => send("working", ctx));
  pi.on("message_end", (event) => {
    const message = event.message;
    if (message?.role !== "assistant") return;
    failed = message.stopReason === "error";
    const text = (message.content ?? []).filter((part) => part.type === "text").map((part) => part.text).join("\n");
    if (text) reply = text;
  });
  // Extension dialogs (a confirm before a tool runs, a question) wait on the user. Outside a run nothing would
  // ever clear them, so only those within one count.
  pi.on("ui_prompt_start", (_event, ctx) => {
    if (running) send("needs_input", ctx);
  });
  pi.on("ui_prompt_end", (_event, ctx) => {
    if (running) send("working", ctx);
  });
  // Settled: no retry, compaction or queued message will carry the run on.
  pi.on("agent_settled", (event, ctx) => {
    running = false;
    if (event.aborted || failed) send("stopped", ctx);
    else send("done", ctx, { last_assistant_message: reply });
  });
  // Pi waits for this before it exits, so nothing still queued is lost. A reload keeps the same session going.
  pi.on("session_shutdown", (event, ctx) => (event.reason === "reload" ? queue : send("session_end", ctx)));
}
