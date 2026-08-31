# UI/UX style guide

How SilkTerm's own interface is meant to look and behave. It covers the menu bar, the right-click menu, the Settings dialog, the About box, and the chrome around the panes. It says nothing about the terminal grid itself, which is the program's output rather than its interface.

This was written by reading what is already built, then tidying the rules until they stop contradicting each other. Where the code and this file disagree, the file is the intent - see [Known deviations](#known-deviations) for the ones that are known and still open.

For prose, comments, naming and Rust conventions, see [`style-guide.md`](../style-guide.md). That guide governs what gets written about the program; this one governs what the program shows.

## Contents

- [Principles](#principles)
- [Words](#words)
- [Menus](#menus)
- [Settings dialog](#settings-dialog)
- [Buttons and prompts](#buttons-and-prompts)
- [Flyover help](#flyover-help)
- [Layout and measurement](#layout-and-measurement)
- [Color roles](#color-roles)
- [Keyboard](#keyboard)
- [Known deviations](#known-deviations)

## Principles

- A terminal is a tool someone stares at all day. Chrome should be quiet, hold still, and stay out of the way of the text.
- Nothing in the interface may block the first frame. Anything slow runs on a worker and appears when it is ready.
- Prefer deriving a piece of state over storing it. If the file on disk or the thing on screen already implies the answer, compute it.
- A control explains itself by its label wherever it can. Flyover help is for the ones that cannot.
- Never surprise. A control that looks standard behaves the standard way, and a familiar keystroke does the familiar thing or is passed to the shell untouched.

## Words

- Sentence case everywhere. Menu titles, menu items, tab names, headings, labels, buttons. Only proper nouns and product names keep their capitals ("PowerShell 7", "Nushell", "SilkTerm").
- No trailing colons on labels. The layout already separates a label from its control.
- No terminating punctuation on a label or a button caption.
- Flyover help is one or two complete sentences, each ending in a period.
- An item that opens a further dialog ends in a single ellipsis character, no space before it: `Settings…`, `About…`, `Save as…`. An item that only asks for confirmation does not.
- Units go on the end of the label, separated by a space, with no brackets: `Opacity %`, `Blur px`, `Blink rate ms`. The value beside the control carries the number alone.
- Keyboard shortcuts shown in a menu go in parentheses at the end of the item, spelled with `+` between every part and no spaces: `Copy (Ctrl+Shift+C)`, `Fullscreen (F11)`.
- Say what a thing is, not what the code calls it. "File or folder", not "Path". "Visibility", not "Alpha".
- Avoid jargon that only a developer would recognize. Where a term is unavoidable and specific to SilkTerm, define it in [`glossary.md`](../glossary.md).

## Menus

- Two menus draw from the same entry list and the same renderer: the menu bar's dropdowns, and the right-click menu over a pane.
- The menu bar is File, Edit, View, Tabs, Panes, Help. A new action goes in the menu whose noun it acts on.
- The right-click menu is the pane's own menu. It repeats the actions that are worth reaching without travelling to the bar, and it may add items that only make sense at the pointer - the two link actions appear only when the click landed on a link.
- Order within a menu: the most-used action first, related actions adjacent, destructive actions last in their group.
- A separator groups; it does not decorate. Every separator must have a reason a reader could name.
- A toggle draws a check mark when it is on. It never changes its caption to describe the other state.
- A submenu is for one homogeneous list that would otherwise swamp its parent - the installed shells are the only case today. It opens beside its parent row, never over it.
- Accelerator letters are unique within one menu, and the first match wins, so a letter spent early is spent. A row that cannot get a distinct letter goes without one rather than stealing one.
- Nothing in a menu is disabled and left visible unless its absence would be more confusing than its being gray.

## Settings dialog

The dialog is declared in `settings_ui.shcl`, which is the file to edit when adding or moving a row.

### Tabs

- Tabs run left to right along the top, on the gutter strip, standing on the line that divides that strip from the rows below.
- A tab holds one subject. Seven is about the ceiling; past that, the subject is probably two subjects.
- A tab's first heading may repeat the tab's own name, in which case it draws nothing. It stays in the declarations because a heading is also what assigns the rows under it to a tab.

### Groups and sub-groups

- A group is a titled section within a tab, separated by a rule and by clear space above it. The heading belongs to what follows it, so the space above a heading is larger than the space below it.
- A sub-group is not declared. It is a row followed by rows at a greater indent. The leader is a real control, not a title.
- Only labels indent. Every control stays in its column, whatever the depth.
- A sub-group's leader is usually the switch that decides whether its members do anything. Members gray out when it is off.

### Rows

- One row edits one setting, and its label names that setting in plain words.
- Row kinds are: heading, toggle, slider, color, text, radio, dropdown, and pair. A new kind needs a reason no existing kind covers.
- A slider carries a number field beside it, and the field is the way to enter an exact value.
- A fraction stored as 0..1 is shown as a whole percent. The file keeps the decimal.
- Every row that can be edited has a revert control at the right edge, which puts the shipped default back.
- A row that is grayed out says why in its flyover help, rather than repeating what it would do.
- Every row must actually write what it edits. A row whose setting is never persisted is worse than no row, because the change appears to take and then vanishes at the next launch.

## Buttons and prompts

- Footer buttons sit at the bottom right, in the order Cancel, Apply, OK, left to right. OK is the default and is marked as such.
- Cancel discards every change. Apply commits without closing. OK commits and closes.
- A button caption is a verb or a standard word, never a sentence.
- A prompt asking for text is a small box with one line of instruction, an entry field with its existing value selected, and Cancel and OK at the bottom right.
- A confirmation names what is about to happen and quotes the thing by name: `Really delete theme "Matrix"?`
- OK is the default in every prompt, including a confirmation to delete. Nothing SilkTerm deletes is unrecoverable enough to justify making the reader reach for the other button every time.
- A red mark is only for removal. Nothing else in the interface is red.

## Flyover help

- One rest delay for every tip in the program. A menu that answered faster than the tab strip would read as a different kind of thing.
- Only controls whose label does not already say what they do get a tip. About a third of rows is the right proportion; if most rows need one, the labels are wrong.
- Text wraps to the panel, so a longer sentence or a larger interface font cannot push it off an edge.
- Placement depends on what is being described. A tip for a row goes under the control and flips above it near the bottom edge. A tip for a menu row goes beside the popup, because a box under the row would cover the rows being chosen between.
- A tip never carries an action, a link, or anything the pointer has to reach.

## Layout and measurement

- Every measurement is in DIP, a CSS pixel at 1/96 inch, and is multiplied by the display's scale factor only at the boundary. Nothing inside the layout is scaled twice.
- Measurements are floors, not fixed sizes. Content that needs more room gets it: a wide label pushes the panel wider, a taller interface font makes rows taller. A number set too small loses to the content; one set too large gives a roomier dialog. Neither can break the layout.
- Chrome sizes off the interface font, not off a constant. Changing the desktop font size must move everything together.
- Text is centered on its visible ink box, not on its line box. Curated single-line labels center on ascender-to-baseline; anything that may carry descenders, such as a path, centers on ascent plus descent.
- A focused boxed control draws exactly one outline.
- A pixel-valued setting steps in whole pixels. Only line height keeps decimals.

## Color roles

Themes ship as a dark and light pair. Ten colors are editable, and each has one job.

- `background`, `foreground`, `cursor`: the terminal grid.
- `highlight`: marks several things at once - the live pane's ring, slider handles, revert icons, the default button. It stays calm because it is everywhere.
- `focus`: marks the one element the keyboard is on. More vivid than `highlight`, and well away from it in hue.
- `menu_background`, `menu_foreground`: the menu bar and its dropdowns.
- `dialog_background`, `dialog_foreground`: the pop-out dialogs.
- `gutter`: chrome that holds no interactive element, such as the strip the dialog's tabs sit on. Recessed against the panel in both modes.

Rules that go with them:

- Do not collapse `highlight` and `focus` back into one color. They answer different questions.
- Chrome defaults are neutral and shared by every built-in theme. A theme may override them, but a terminal palette that repainted the menu bar would fight the desktop.
- Any two colors drawn on each other need a contrast check, not a taste check. A harmonious pair of hues can share a luminance and become unreadable.

## Keyboard

- The shell gets the keystroke unless the interface has a specific claim on it. When in doubt, it goes to the shell.
- Program shortcuts take Ctrl+Shift where the plain Ctrl form belongs to the shell: Ctrl+Shift+C, Ctrl+Shift+V, Ctrl+Shift+T, Ctrl+Shift+W.
- Inside a dialog, Tab and Shift+Tab move focus, Ctrl+Tab and Ctrl+PgUp/PgDn change tab, Enter is OK and Escape is Cancel.
- Every action reachable by mouse is reachable by keyboard. The reverse does not have to hold.
- A key that reaches a dialog never also reaches the terminal underneath.

## Known deviations

Things the built interface does differently from the rules above. Each is a small work item rather than a design question.

- `Paste Selection` keeps a capital S so that the accelerator has a letter to land on. Documented as an exception, but a better fix would free a letter elsewhere.
- `Copy on select` sits at the bottom of the Cursor tab, which is not where its subject is. It was asked for there and a test pins it, so it stays until that changes.
