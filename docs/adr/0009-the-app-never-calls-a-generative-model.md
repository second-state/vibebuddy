---
status: accepted
---

# The App never calls a generative model; custom looks are imported

Users want to make their own Character, for example by having an image model draw it. We decided the App does not call any language, image or speech model. Instead we publish a sprite-sheet template and a prompt; the user makes the image with whatever tool they like and drops it into the App, which validates it, reduces its colors and writes it to the box.

## Considered options

- **Generate inside the App with the user's API key.** One-click, but it brings key management, provider choice, cost, failures and content moderation into the App, and the resulting sheet still has to pass the same validation.
- **Generate through a VibeBuddy server.** That adds accounts, billing and abuse handling, and makes a local, open-source tool depend on a service.

## Consequences

The hard part becomes the template: it has to be clear enough that a general image model fills it in correctly, and the importer has to reject or repair what comes back. A custom Character borrows its voice, persona and lines from a preset Character, since making those would need TTS and a language model.
