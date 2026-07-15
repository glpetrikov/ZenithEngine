using ZeroEngine;

public class UITest : ZEScript
{
    // Fields
    private Text uiText;
    private Button uiButton;
    private Bar uiBar;

    [ZeroField] private UInt32 hp = 75;
    [ZeroField] private UInt32 maxHp = 100;

    public override void OnCreate()
    {
        uiText = GetComponent<Text>();
        uiButton = GetComponent<Button>();
        uiBar = GetComponent<Bar>();
    }

    public override void OnStart()
    {
        uiBar.SetLabel($"{(int)hp}/{(int)maxHp}");
        uiBar.SetColor(Color.Green);
        uiBar.SetBgColor(Color.DarkGray);

        uiText.SetText($"{(int)hp}/{(int)maxHp}");
        uiText.SetColor(Color.Green);
        uiText.SetFontSize(18f);

        uiButton.SetText("I'm Button");
        uiButton.SetColor(Color.DarkGray);
        uiButton.SetHoverColor(Color.Gray);
        uiButton.SetPressedColor(Color.Red);
        uiButton.SetOnClick(() => { uiText.SetText("Button Clicked! from callback"); });
    }

    public override void OnUpdate()
    {
        // if (uiButton.IsClicked())
        // {
        //     uiText.SetText("Button Clicked! from if");
        // }
    }

    public override void OnFixedUpdate()
    {
    }
}
