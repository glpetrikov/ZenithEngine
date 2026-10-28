# Chapter 1

This is just a joke.

## Cool code example
Code below by Claude.
```rust
use zenith_api::{
    input::{Input, ActionState},
    physics::Physics,
    time::Time,
    transform::Transform,
    ZEScript,
};

const WALK_SPEED: f32 = 4.0;
const SPRINT_MULTIPLIER: f32 = 1.6;
const CROUCH_MULTIPLIER: f32 = 0.5;
const JUMP_FORCE: f32 = 6.5;
const GRAVITY: f32 = -18.0;
// Coyote time: keeps a jump valid for a short window after walking off a
// ledge, since players almost always press jump a few frames too late and
// a hard "must be grounded exactly this frame" check reads as unresponsive.
const COYOTE_TIME: f32 = 0.15;

#[derive(Default)]
pub struct PlayerController {
    velocity_y: f32,
    coyote_timer: f32,
}

impl ZEScript for PlayerController {
    fn on_start(&mut self) {
        Input::add_action("MoveForward", &["W"]);
        Input::add_action("MoveBack", &["S"]);
        Input::add_action("MoveLeft", &["A"]);
        Input::add_action("MoveRight", &["D"]);
        Input::add_action("Jump", &["Space"]);
        Input::add_action("Sprint", &["LeftShift"]);
        Input::add_action("Crouch", &["LeftControl"]);
    }

    fn on_update(&mut self, time: &Time) {
        let mut transform = self.get_component_mut::<Transform>();

        // The countdown resets every grounded frame instead of only once on
        // landing, so the window always covers "the last N seconds spent on
        // the ground", not just the instant contact was lost.
        if Physics::is_grounded(self.entity()) {
            self.coyote_timer = COYOTE_TIME;
            self.velocity_y = 0.0;
        } else {
            self.coyote_timer -= time.delta_seconds();
        }

        if Input::was_action_just_pressed("Jump") && self.coyote_timer > 0.0 {
            self.velocity_y = JUMP_FORCE;
            // Consumed immediately so the same airborne window can't grant
            // a second jump if the player mashes the button.
            self.coyote_timer = 0.0;
        }

        let mut direction = glam::Vec3::ZERO;
        if Input::is_action_pressed("MoveForward") { direction.z += 1.0; }
        if Input::is_action_pressed("MoveBack") { direction.z -= 1.0; }
        if Input::is_action_pressed("MoveRight") { direction.x += 1.0; }
        if Input::is_action_pressed("MoveLeft") { direction.x -= 1.0; }
        let direction = direction.normalize_or_zero();

        let speed = if Input::is_action_pressed("Sprint") {
            WALK_SPEED * SPRINT_MULTIPLIER
        } else if Input::is_action_pressed("Crouch") {
            WALK_SPEED * CROUCH_MULTIPLIER
        } else {
            WALK_SPEED
        };

        self.velocity_y += GRAVITY * time.delta_seconds();

        let motion = direction * speed * time.delta_seconds()
            + glam::Vec3::Y * self.velocity_y * time.delta_seconds();
        transform.position += motion;
    }
}
```

it's production ready, AAA, Player controller for your game on Rust for ZenithEngine
