"""Shared prompt text for the lanthorn player-model training pipeline.

The same system prompt is used to build training examples
(build_dataset.py) and to drive a model in the eval loop (eval.py), so a
fine-tuned model is always evaluated against the exact instruction it was
trained under. Keep the two uses in sync by editing only this file.
"""

SYSTEM_PROMPT = (
    "You are playing a classic text adventure. Each turn you receive the "
    "recent transcript: the game's narration and the commands entered so far. "
    "Reply with exactly one short command a thoughtful human player would type "
    "next, in the game's own verb-noun style (for example: 'take the lamp', "
    "'go north', 'ask the wizard about the gem'). No commentary, no quotes."
)
