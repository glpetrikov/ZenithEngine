# Input

`zenith_input` keeps the state of the keyboard, the mouse and the gamepads and
answers questions about it. It is a plain value, `Input`, and in the engine it
is a resource of the world:

```rust,ignore
// Anywhere you have the world:
if world.input().is_key_pressed(ZKeyCode::W) { /* ... */ }

// In a system:
fn walk(input: Res<Input>, time: Res<Time>) {
    if input.is_action_pressed("Sprint") { /* ... */ }
}
```

`World::new` creates it, `update_systems(dt)` ends the frame after the systems
have run, and clearing the world for a new level keeps it: input belongs to the
player, not to the level. The runtime feeds it from the window and from the
gamepads; Escape closes the runtime through it. Mouse positions, movements and
sticks are `glam::Vec2` (`zenith_input` re-exports it as `Vec2`).

## Keyboard

Keys are **physical**: `ZKeyCode::W` is the key in the `W` position of a US
keyboard, which is the key under the finger of a player on any layout (`WASD`
works on another one too). The names are those of `winit` with friendlier
spellings: `A`..`Z`, `Digit0`..`Digit9`, `Space`, `Enter`, `Escape`,
`LeftShift`, `RightControl`, `Up`, `Numpad0`, `F1`..`F35`, and so on. In text
(`Input.toml`, serde) the case does not matter, and `left_shift`, `Esc`, `Ctrl`
and the `winit` names (`KeyW`, `ShiftLeft`) are accepted.

| Call                        | Meaning                                                                                                            |
| --------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `is_key_pressed(key)`       | held down                                                                                                          |
| `is_key_just_pressed(key)`  | went down in this frame                                                                                            |
| `is_key_just_released(key)` | came up in this frame                                                                                              |
| `key_held_for(key)`         | seconds it has been held (`None` if it is not)                                                                     |
| `pressed_keys()`            | every key that is held                                                                                             |
| `text()`                    | characters typed in this frame (for text fields; layouts and the keyboard repeat apply, control characters do not) |

A key that repeats while held is not pressed again. If the window loses the
focus, every key and mouse button is released (their release will never be
seen), so a character does not run on after alt-tab.

## Mouse

`is_mouse_pressed(ZMouseButton::Left)`, `is_mouse_just_pressed`,
`is_mouse_just_released`; `mouse_position()` (`Option<Vec2>`, physical pixels,
`None` while the cursor is outside); `mouse_delta()` (`Vec2`, movement in this
frame, times the sensitivity of `Input.toml`); `scroll()` (`Vec2`, wheel, in
lines). With an ordinary cursor `mouse_delta` is the movement of the cursor in
pixels, so one sensitivity means the same thing everywhere. A locked cursor
(`CursorMode::Locked`, below) does not move, so then the raw movement of the
mouse is used (its own units, steady from frame to frame).

### The cursor

```rust,ignore
input.set_cursor_mode(CursorMode::Locked);   // Normal | Hidden | Locked
```

`Locked` hides the cursor and keeps it in the window, for a camera that turns
with the mouse (use `mouse_delta`). `Input` only records the request; the code
that owns the window carries it out: the runtime does it every frame, with
`zenith_input::apply_cursor_mode(&window, mode)` (feature `winit`), which is
there for your own loop too. In a browser the page needs a click before the
cursor can be locked, and a failure is only logged.

## Gamepads

A gamepad is an object you ask for by its number; `None` if there is no such
pad:

```rust,ignore
for pad in input.list_gamepads() {                // GamepadInfo { id, name, kind }
    println!("{}: {} ({})", pad.id, pad.name, pad.kind);
}
if let Some(pad) = input.gamepad(0) {
    pad.name();               // "Xbox Controller"
    pad.kind();               // ZGamepadKind::Xbox | PlayStation | Nintendo | Generic
    pad.power_info();         // ZPowerInfo::Discharging(40), Charging(..), Charged, Wired, Unknown
    pad.is_ff_supported();    // can it vibrate?
    pad.has_standard_mapping();
    pad.is_pressed(ZGamePad::South);
    pad.stick(ZStick::Left);  // Vec2
    pad.axis(ZGamePadAxis::RightTrigger);
}
// The short forms:
input.is_gamepad_pressed(ZGamePad::South, 0);
input.gamepad_stick(ZStick::Left, 0);
```

Buttons are named by place: `South` is the bottom face button (A on Xbox, cross
on PlayStation), then `East`, `West`, `North`, `LeftBumper`, `RightBumper`,
`LeftTrigger`, `RightTrigger`, `Select`, `Start`, `Mode`, `LeftStick`,
`RightStick` and the `DPad*` four. `kind()` guesses the family from the maker
of the pad, for choosing the pictures of the buttons ("press Cross" or "press
A"). Gamepads have numbers: the first free one (`0`, `1`, ...) goes to the next
pad that is connected, and a number is reused once its pad is unplugged.

A **stick** is judged as a whole: a push that is short on both axes is no push
(a round dead zone), and the range outside the dead zone is stretched to the
full length `1`, so there is no jump where the dead zone ends. `stick()` gives
the vector (never longer than `1`; `.normalize_or_zero()` for a direction
only). Triggers go from `0.0` to `1.0` with their own, smaller, dead zone. The
response curve of `Input.toml` (`gamepad.curve`) bends the length of the push
after that: `1` is linear, `2` makes small pushes finer.

`power_info()` is the battery. The system has no event for it, so the runtime
looks every second; in a browser it is always `Unknown`. A pad with
`has_standard_mapping() == false` has no known button layout (the runtime logs
a warning when it is connected): add one with
`Gamepads::with_mappings(sdl_text)` or the `SDL_GAMECONTROLLERCONFIG`
environment variable.

### Vibration

```rust,ignore
let effect = RumbleEffect::new().strong(0.8).weak(0.3).for_ms(250);
input.rumble(0, effect)?;                      // Err if the pad cannot vibrate
input.gamepad_mut(0).map(|mut pad| pad.rumble(effect));   // the same, on the object
input.stop_rumble(0);
```

The strong and the weak motor each take `0.0..=1.0`. The call returns an error
(`Unsupported`) at once when the pad is not there or does not support force
feedback (check `gamepad(0)?.is_ff_supported()` to know before; it never does
in a browser), and `InvalidSettings` for an effect that plays nothing. The
vibration itself is carried out by `Gamepads::poll`.

### Real gamepads

Real pads come from `gilrs` (the `gamepad` feature, on in the runtime; it uses
the Gamepad API in a browser): `Gamepads::new()?` and `gamepads.poll(&mut input)`
once per frame. `poll` reads the events, carries out the vibrations the game
asked for and updates the batteries. It never waits for an event
(`Gilrs::next_event_blocking` panics in a browser, and `clippy.toml` forbids
it).

## Actions

An action is a name the game asks for, bound to inputs, so that the code says
`Jump` and the player decides which button that is.

```rust,ignore
input.add_action("Jump", [Binding::from(ZKeyCode::Space), ZGamePad::South.into()]);
input.add_action("Save", [Binding::from(ZKeyCode::S).with(ZKeyCode::LeftControl)]);

input.is_action_pressed("Jump");
input.is_action_just_pressed("Jump");
input.is_action_just_released("Jump");
```

**A button action is pressed when any of its bindings is.** `W`, `S`, `T`, `B`
in one action: one key is enough, and that is all it says. A binding is a key,
a mouse button, a gamepad button (of any pad) or an axis (of the pad that
pushes it furthest, pressed beyond `gamepad.press_threshold`). `Binding::with`
makes a chord: the binding counts only while the other input is held
(`Ctrl+S`). `Binding::on_pad(n)` (`Gamepad:South@1` in text) makes a gamepad
binding read only pad number `n`, so two players on two pads have a binding
each (`Jump` for player one on `@0`, for player two on `@1`).

For something that is a number rather than a yes or no, two more calls:

* `action_value(name)` is the sum of the bindings, `-1..=1`. A binding
  multiplies its input by a factor (`.negated()` is `-1`), so `D` and a negated
  `A` together are one value that is `1` for D, `-1` for A, and `0` for both.
* `action_vector2(name)` is a **vector action**, separate from the button
  actions, with a list of bindings for `x` and for `y`:

```rust,ignore
input.add_vector_action("Move", VectorAction {
    x: vec![Binding::from(ZKeyCode::D), Binding::from(ZKeyCode::A).negated(),
            ZGamePadAxis::LeftStickX.into()],
    y: vec![Binding::from(ZKeyCode::W), Binding::from(ZKeyCode::S).negated(),
            ZGamePadAxis::LeftStickY.into()],
    normalize: true,
});
let direction: Vec2 = input.action_vector2("Move");
```

With `normalize` (the default) the vector is never longer than `1`, so two keys
at once (a diagonal) are no faster than one; a stick, which is analog, keeps
its own length. Without it each axis is limited on its own.

"Just pressed" and "just released" are noted when the event happens, not when
somebody asks, so a tap that begins and ends between two frames (a low frame
rate, a lag) is seen as both a press and a release, for a key and for an action
alike. A pad that is unplugged while a button is held releases the button too
(`is_gamepad_just_released` says so for that frame, though the pad is gone).

### Holding and buffering

`action_held_for(name)` is the number of seconds the action has been pressed.
`action_pressed_within("Jump", 0.15)` is `true` if it was pressed in the last
`0.15` seconds and not used up since: a jump pressed a moment before landing
still counts. When the game uses the press, `consume_action("Jump")` stops it
from counting again. Time comes from `end_frame(dt)`; `input.time()` is the
sum, kept in `f64` so that a session of days (an editor left open) still
measures a single frame.

### Changing bindings

For an option menu: `set_bindings`, `rebind(name, index, binding)`,
`add_binding`, `remove_binding` and `reset_action` work on the actions that
exist. To ask the player for a key:

```rust,ignore
input.listen_for_binding();            // "press the key you want..."
// ...on a later frame:
if let Some(binding) = input.take_captured() {   // a key, a mouse button, a pad button,
    input.rebind("Jump", 0, binding)?;           // or an axis pushed far (negated if pushed
}                                                // the other way)
```

For a chord, hold Ctrl, Shift, Alt or Super first and then press the key
(`Ctrl+S`; also `Ctrl+click`); a modifier pressed and let go on its own is taken
as it is. The press still counts for the game too, so listen in a menu;
`cancel_listening` stops waiting. What the player changed is saved apart from the project:
`input.overrides_toml()` is the text of a file with only the actions that
differ from how they were defined (the same format as `Input.toml`), and
`input.load_overrides(&text)` puts it back (actions the game no longer has are
skipped, and the settings in the file are not used). Where to keep it (the data
folder of the player) is up to the game.

## `Settings/Input.toml`

Settings and actions can come from a file of the project. It is in `Settings/`,
so it goes into builds. Read it with the project and give the text to `Input`:

```rust,ignore
let text = project.read_string(&ZPath::new("Settings/Input.toml")?)?;
world.input_mut().load_settings(&text)?;
```

```toml
[gamepad]
dead_zone = 0.15          # sticks: pushes within this radius are 0
trigger_dead_zone = 0.05
press_threshold = 0.5     # an axis counts as pressed beyond this
curve = 1.0               # 1 is linear, 2 makes small pushes finer (0.1 to 8)

[mouse]
sensitivity = 1.0         # multiplies mouse_delta
invert_y = false

[[action]]                # pressed when any of its bindings is
name = "Jump"
bindings = ["Key:Space", "Gamepad:South"]

[[action]]
name = "Save"
bindings = ["Key:LeftControl+Key:S"]

[[vector]]                # two values, for walking and looking
name = "Move"
x = ["Key:D", "-Key:A", "Axis:LeftStickX"]
y = ["Key:W", "-Key:S", "Axis:LeftStickY"]
normalize = true
```

A binding is `Key:<name>`, `Mouse:<Left|Right|Middle|Back|Forward|Other<n>>`,
`Gamepad:<button>` or `Axis:<axis>`, with a `-` in front to negate it and `+`
between inputs for a chord. Anything not written keeps its default. A file with
a mistake (an unknown key, a number out of range, a binding that cannot be
read, an action defined twice) is refused as a whole with `InvalidSettings` and
changes nothing; actions that are already defined and not in the file stay. The
actions of the file become the ones that `reset_action` goes back to.

## Where events come from

`Input` knows nothing about windows or devices. Everything reaches it as an
`InputEvent` through `input.apply(event)`: `KeyDown`, `MouseMoved`, `Text`,
`Focus`, `GamepadButton`, `GamepadAxis`, `GamepadPower` and so on. The adapters
only translate:

| Source                             | How                                                                         |
| ---------------------------------- | --------------------------------------------------------------------------- |
| a `winit` window (feature `winit`) | `input.handle_window_event(&event)` and `input.handle_device_event(&event)` |
| real gamepads (feature `gamepad`)  | `Gamepads::poll(&mut input)`                                                |
| anything else                      | build the events yourself                                                   |

That last row is what makes the rest testable without hardware: a test, a
recording or an on-screen emulated gamepad just applies events. An event for a
gamepad number that is not connected yet connects it; `GamepadConnected` also
says whether the pad can vibrate, so an emulated pad can pretend to.

```rust,ignore
use zenith_input::{Input, InputEvent, ZGamePad, ZGamePadAxis};

let mut input = Input::default();
input.apply(InputEvent::GamepadButton { id: 0, button: ZGamePad::South, pressed: true });
input.apply(InputEvent::GamepadAxis { id: 0, axis: ZGamePadAxis::LeftStickX, value: -1.0 });
assert!(input.is_gamepad_pressed(ZGamePad::South, 0));
assert!(input.gamepad_axis(ZGamePadAxis::LeftStickX, 0) < -0.99);
```

## Recording and playing back

```rust,ignore
input.start_recording();
// ... play: every event is written down with the frame it came in ...
let recording = input.stop_recording();
std::fs::write("play.json", recording.to_json()?)?;

// Later, in a test or a demo:
let mut replay = Replay::new(Recording::from_json(&std::fs::read_to_string("play.json")?)?);
while let Some(dt) = replay.advance(&mut world.input_mut()) {
    world.update_systems(dt);        // runs the frame and ends it
}
```

A `Recording` holds the events and how long each frame lasted. Settings,
actions and the cursor mode are not events, so set the `Input` up the same way
before playing back.

## The frame

Events arrive between frames and are applied at once; "just", the mouse
movement and the text describe what happened since the last `end_frame(dt)`.
`World::update_systems(dt)` runs the systems and then calls it, so systems see
everything that happened since the previous frame, and nothing is seen twice.
If you use `Input` without a world, call `end_frame(dt)` yourself, once per
frame.
