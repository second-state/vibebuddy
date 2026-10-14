# VibeBuddy, the robot

The default Character. Its look is drawn by the firmware itself, so there is no `look.png` and no `pack.bin` here: the App writes the robot as a pack without a look, with its own voice: the built-in voice's five fixed lines (`voices/jessica/`, packed as `robot`).

`persona.md` is how it talks when it answers a voice command (docs/resident-session.md, "Persona and language"); it has no `lines.tsv`, as its lines are the five fixed ones.

`face.png` is the robot on its idle frame, rendered by the firmware's host preview and halved, for the Character cards.
