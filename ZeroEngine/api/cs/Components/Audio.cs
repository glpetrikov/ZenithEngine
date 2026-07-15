namespace ZeroEngine;

public sealed unsafe class Audio : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Audio;

    public void Play()
    {
        EngineAPI.Current->play_audio(EntityId);
    }

    public void Stop()
    {
        EngineAPI.Current->stop_audio(EntityId);
    }

    public void SetVolume(float volume)
    {
        EngineAPI.Current->set_audio_volume(EntityId, volume);
    }

    public bool IsPlaying()
    {
        return EngineAPI.Current->is_audio_playing(EntityId);
    }
}
