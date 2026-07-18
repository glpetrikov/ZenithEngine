using ZeroEngine;

namespace Sandbox;

public class Script : ZEScript
{
    [ZeroField]
    private float speed = 0.5f;

    // Matches the Circle collider's radius in the scene; used to find the "feet" point for
    // the ground raycast, since scripts can't query a collider's shape/size directly.
    [ZeroField]
    private float colliderRadius = 0.5f;

    [ZeroField]
    private float jumpForce = 5.0f;

    private Rigidbody? rb;
    private Transform? transform;
    private Audio? jumpAudio;
    private bool moveLeft;
    private bool moveRight;

    private bool jumpRequested;

    public override void OnStart()
    {
        rb = GetComponent<Rigidbody>();
        transform = GetComponent<Transform>();
        jumpAudio = GetComponent<Audio>();
    }

    public override void OnUpdate()
    {
        moveLeft = Input.IsKeyPressed(KeyCode.A);
        moveRight = Input.IsKeyPressed(KeyCode.D);

        if (Input.IsKeyJustPressed(KeyCode.Space))
        {
            jumpRequested = true;
        }

        if (Input.IsKeyJustPressed(KeyCode.R))
        {
            Log.Info("Reloading Scene...");
            Scene.Reload();
        }
        if (Input.IsKeyJustPressed(KeyCode.M))
        {
            Log.Info("Loading Main Scene...");
            Scene.LoadMain();
        }
        if (Input.IsKeyJustPressed(KeyCode.L))
        {
            Log.Info("Loading OtherScene...");
            Scene.Load("OtherScene");
        }
    }

    public override void OnFixedUpdate()
    {
        if (rb is null || transform is null || jumpAudio is null) return;

        var maxVelocity = new Vector2(20.0f, 20.0f);

        if (moveLeft)
        {
            rb.Add2DForceWithMax(new Vector2(-speed, 0.0f), maxVelocity, ForceMode.Force);
        }
        else if (moveRight)
        {
            rb.Add2DForceWithMax(new Vector2(speed, 0.0f), maxVelocity, ForceMode.Force);
        }

        if (jumpRequested)
        {
            // Ground check: cast a short ray from just below the feet straight down. Starting
            // a hair outside the character's own collider (rather than exactly on its boundary)
            // avoids a degenerate self-hit at t=0; the remaining distance is what stops the
            // jump from re-triggering every fixed step while airborne.
            const float skin = 0.02f;
            var feet = transform.Position - new Vector2(0.0f, colliderRadius + skin);
            bool grounded = Physics.Raycast(feet, new Vector2(0.0f, -1.0f), 0.1f, out _);

            if (grounded)
            {
                rb.Add2DForceWithMax(new Vector2(0.0f, jumpForce), maxVelocity, ForceMode.Impulse);
                jumpAudio.Play();
            }

            jumpRequested = false;
        }
    }
}
