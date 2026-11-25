# Reads the assistant reply from stdin and speaks it with a local Windows voice.
# Used by voice/wakeword.py as --tts-command; runs entirely offline (SAPI).
# Some background sessions deny SelectVoice(); fall back to the default voice,
# then to an audible local tone so the listener never depends on TTS success.
$ErrorActionPreference = 'Continue'
Add-Type -AssemblyName System.Speech

$text = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($text)) { exit 0 }

try {
    $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
    $spanish = @($synth.GetInstalledVoices() |
        Where-Object { $_.Enabled -and $_.VoiceInfo.Culture.Name -like 'es-*' } |
        Select-Object -First 1)
    if ($spanish.Count -gt 0) {
        try {
            $synth.SelectVoice($spanish[0].VoiceInfo.Name)
        } catch {
            # Background/security policy may deny explicit selection; the
            # default system voice still speaks the text.
            Write-Output "[tts] Spanish voice selection denied: $($_.Exception.Message)"
        }
    }
    $synth.Speak($text.Trim())
    $synth.Dispose()
    exit 0
} catch {
    Write-Output "[tts] speech failed: $($_.Exception.Message)"
    try {
        [Console]::Beep(880, 160)
        [Console]::Beep(1180, 160)
    } catch {
        # no audio device available at all
    }
    exit 1
}
