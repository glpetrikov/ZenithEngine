using ZeroEngine;

public class ScriptTest : ZEScript
{
    [ZeroField]
    private Vector3 position = new Vector3(3, 6, 1);
    // Fields


    public override void OnUpdate()
    {
        if (Input.IsKeyJustPressed(KeyCode.E))
        {
            Application.Exit();
        }

        if (Input.IsMouseButtonJustPressed(MouseButton.Left))
        {
            Console.WriteLine("Left Mouse Button clicked!");
        }
    }

}
