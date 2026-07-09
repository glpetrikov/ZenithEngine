namespace ZeroEngine;

public static unsafe class Application
{
    public static void Exit()
    {
        Quit();
    }
    public static void Quit()
    {
        EngineAPI.Current->quit_game();
    }
}
