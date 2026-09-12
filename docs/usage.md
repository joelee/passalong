# Usage

> Draft. Each command's section is written when the command lands
> (PLAN-00001 STEP-08 to STEP-13) and completed in STEP-14.

## Clipboard support

Clipboard text works on macOS and on Linux under X11 or a Wayland compositor
that supports the `wlr-data-control` protocol, such as Hyprland or Sway.

On Linux the clipboard's content belongs to the program that set it. When
`passalong load` copies text to the clipboard and exits, the text survives
only if a clipboard manager takes it over; passalong waits up to 2 seconds
for one. Without a clipboard manager, load into a file instead.
