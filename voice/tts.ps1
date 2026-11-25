# Reads the assistant reply from stdin and speaks it with a local Windows voice.
# Used by voice/wakeword.py as --tts-command; runs entirely offline (SAPI).
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Speech

$text = [Console]::In.ReadToEnd()
if ([string]::IsNullOrWhiteSpace($text)) { exit 0 }

$synth = New-Object System.Speech.Synthesis.SpeechSynthesizer
$synth.Rate = 0
$synth.Volume = 100
$spanish = @($synth.GetInstalledVoices() |
    Where-Object { $_.VoiceInfo.Culture.Name -like 'es-*' } |
    Select-Object -First 1)
if ($spanish.Count -gt 0) {
    $synth.SelectVoice($spanish[0].VoiceInfo.Name)
}
$synth.Speak($text.Trim())
