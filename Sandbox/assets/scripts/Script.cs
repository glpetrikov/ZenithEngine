using ZeroEngine;

namespace Sandbox;

public class Script : ZEScript
{
    [ZeroField]
    private float speed = 0.5f;

    private Rigidbody? rb;
    private bool moveLeft;
    private bool moveRight;

    private bool jumpRequested;

    public override void OnStart()
    {
        rb = GetComponent<Rigidbody>();
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
        if (rb is null) return;

        const float jumpForce = 2.0f;
        var maxVelocity = new Vector2(2.5f, 5.0f);

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
            rb.Add2DForceWithMax(new Vector2(0.0f, jumpForce), maxVelocity, ForceMode.Impulse);
            jumpRequested = false;
        }
    }
}
