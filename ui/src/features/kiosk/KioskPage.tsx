import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useSearchParams } from 'react-router-dom';
import {
  AudioLines,
  CloudOff,
  CloudSun,
  Home,
  Mic,
  Music2,
  Pause,
  Play,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Square,
  Volume2,
  Wifi,
  X,
} from 'lucide-react';
import {
  detectKioskWake,
  kioskApi,
  synthesizeKioskSpeech,
  transcribeKioskAudio,
  transcribeKioskAudioFast,
} from '@/api/kiosk';
import type { KioskHistoryEntry } from '@/api/kiosk';
import { profilesApi } from '@/api/profiles';
import { isKioskEvent } from '@/api/types';
import type {
  AgentActivity,
  AvatarState,
  KioskEvent,
  KioskScene,
  KioskStateResponse,
  KioskWeather,
  MusicState,
  NetworkHealth,
  Profile,
  ToolTraceEntry,
  TrackingPoint,
} from '@/api/types';
import { ReconnectingSocket } from '@/api/ws';
import type { SocketStatus } from '@/api/ws';
import { BoopAvatar } from '@/components/BoopAvatar';
import { Spinner } from '@/components/Spinner';
import { StatusPill } from '@/components/StatusPill';
import { errorMessageKey, useI18n } from '@/i18n/I18nProvider';
import type { DictKey } from '@/i18n/dictionary';
import { cn } from '@/lib/cn';
import { formatDate, formatTime } from '@/lib/format';
import { readStored } from '@/lib/preferences';
import { useInterval, useNow } from '@/lib/useInterval';
import { useToast } from '@/state/ToastProvider';
import { DEMO_KIOSK_STATE, DEMO_PROFILES } from './demo';

const ACTIVITY_KEYS: Record<AgentActivity, DictKey> = {
  idle: 'kiosk.agent.idle',
  listening: 'kiosk.agent.listening',
  thinking: 'kiosk.agent.thinking',
  speaking: 'kiosk.agent.speaking',
  error: 'kiosk.agent.error',
};

const DEMO_STATES: AvatarState[] = [
  'idle',
  'tracking',
  'listening',
  'thinking',
  'speaking',
  'profile_switch',
];

const EMPTY_MUSIC: MusicState = {
  playing: false,
  track: null,
  artist: null,
  volume: 60,
  sink: null,
};

const EMPTY_NETWORK: NetworkHealth = { online: false, dns_ok: false, latency_ms: null };

const DEFAULT_IDLE_SECONDS = 120;

function idleTimeoutMs(): number {
  const stored = readStored('hs.kioskIdleSeconds');
  const parsed = stored !== null ? Number(stored) : Number.NaN;
  if (Number.isFinite(parsed) && parsed >= 10 && parsed <= 3_600) return parsed * 1000;
  return DEFAULT_IDLE_SECONDS * 1000;
}

function avatarSizeForViewport(): number {
  if (typeof window === 'undefined') return 320;
  const height = window.innerHeight;
  return Math.round(Math.min(420, Math.max(220, height * 0.42)));
}

interface LiveState {
  profile: Profile | null;
  music: MusicState;
  scenes: KioskScene[];
  network: NetworkHealth;
  agentActivity: AgentActivity;
}

/**
 * Wall kiosk experience: full-screen avatar, clock, music, scenes and voice UI.
 * Demo mode (`?demo=1` or `VITE_DEMO=1`) renders labelled demo data without
 * touching the daemon.
 */
interface ChatEntry {
  id: number;
  role: 'user' | 'assistant';
  text: string;
}

export function KioskPage(): React.JSX.Element {
  const { t, language } = useI18n();
  const toast = useToast();
  const pushToast = toast.push;
  const [params] = useSearchParams();
  const isDemo = params.get('demo') === '1' || import.meta.env.VITE_DEMO === '1';
  const now = useNow(1_000);
  const [avatarSize, setAvatarSize] = useState(avatarSizeForViewport);
  const [weather, setWeather] = useState<KioskWeather | null>(null);
  const [automationOpen, setAutomationOpen] = useState(false);

  const initial = isDemo ? DEMO_KIOSK_STATE : null;
  const [live, setLive] = useState<LiveState>({
    profile: initial?.profile ?? null,
    music: initial?.music ?? EMPTY_MUSIC,
    scenes: initial?.scenes ?? [],
    network: initial?.network ?? EMPTY_NETWORK,
    agentActivity: initial?.agent_activity ?? 'idle',
  });
  const [assistantName, setAssistantName] = useState(initial?.assistant_name ?? 'Bober');
  const [avatarState, setAvatarState] = useState<AvatarState>('idle');
  const [tracking, setTracking] = useState<TrackingPoint | null>(null);
  const [wsStatus, setWsStatus] = useState<SocketStatus>(isDemo ? 'open' : 'connecting');
  const [volume, setVolume] = useState(initial?.music.volume ?? 60);
  const [busy, setBusy] = useState<string | null>(null);
  const [profiles, setProfiles] = useState<Profile[]>(isDemo ? DEMO_PROFILES : []);
  const [profileSheetOpen, setProfileSheetOpen] = useState(false);
  const [talkOpen, setTalkOpen] = useState(false);
  const [queryText, setQueryText] = useState('');
  const [chatLog, setChatLog] = useState<ChatEntry[]>([]);
  const [agentTrace, setAgentTrace] = useState<ToolTraceEntry[]>([]);
  const [conversation, setConversation] = useState<KioskHistoryEntry[]>([]);
  const [recording, setRecording] = useState(false);
  const [transcribing, setTranscribing] = useState(false);
  const [continuous, setContinuous] = useState(false);
  const [micState, setMicState] = useState<
    'unknown' | 'granted' | 'prompt' | 'denied' | 'unsupported'
  >('unknown');
  const [micError, setMicError] = useState<string | null>(null);
  const continuousRef = useRef(false);
  const streamRef = useRef<MediaStream | null>(null);
  const recorderRef = useRef<MediaRecorder | null>(null);
  const chunksRef = useRef<Blob[]>([]);
  const chatTimer = useRef(0);
  const chatSequence = useRef(0);
  const queryInput = useRef<HTMLInputElement | null>(null);
  const [querying, setQuerying] = useState(false);
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<number>(0);
  const lastActivity = useRef(Date.now());
  const lastEventAt = useRef(0);
  const speechTimer = useRef(0);
  const speechAudio = useRef<HTMLAudioElement | null>(null);
  const speechUrl = useRef<string | null>(null);
  const lastSpoken = useRef<{ text: string; at: number }>({ text: '', at: 0 });

  useEffect(() => {
    let cancelled = false;
    const check = async (): Promise<void> => {
      if (
        typeof window === 'undefined' ||
        !window.isSecureContext ||
        navigator.mediaDevices?.getUserMedia === undefined
      ) {
        if (!cancelled) setMicState('unsupported');
        return;
      }
      try {
        const status = await navigator.permissions.query({
          name: 'microphone' as unknown as PermissionName,
        });
        if (cancelled) return;
        setMicState(status.state as 'granted' | 'prompt' | 'denied');
        status.onchange = () => {
          if (!cancelled) {
            setMicState(status.state as 'granted' | 'prompt' | 'denied');
          }
        };
      } catch {
        if (!cancelled) setMicState('prompt');
      }
    };
    void check();
    return () => {
      cancelled = true;
    };
  }, []);

  /* Clock / viewport / idle ------------------------------------------------- */

  useEffect(() => {
    if (isDemo) return undefined;
    let cancelled = false;
    const load = (): void => {
      void kioskApi
        .weather('Lima')
        .then((snapshot) => {
          if (!cancelled) setWeather(snapshot);
        })
        .catch(() => undefined);
    };
    load();
    const timer = window.setInterval(load, 10 * 60 * 1000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [isDemo]);

  useEffect(() => {
    if (!automationOpen) return undefined;
    const timer = window.setTimeout(() => setAutomationOpen(false), 30_000);
    return () => window.clearTimeout(timer);
  }, [automationOpen]);

  useEffect(() => {
    const onResize = (): void => setAvatarSize(avatarSizeForViewport());
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  }, []);

  useEffect(
    () => () => {
      window.clearTimeout(chatTimer.current);
    },
    [],
  );

  const wake = useCallback(() => {
    lastActivity.current = Date.now();
    setIdle(false);
  }, []);

  useEffect(() => {
    idleTimer.current = idleTimeoutMs();
    const onActivity = (): void => wake();
    const events: Array<keyof WindowEventMap> = ['pointerdown', 'pointermove', 'keydown', 'touchstart'];
    for (const event of events) window.addEventListener(event, onActivity, { passive: true });
    return () => {
      for (const event of events) window.removeEventListener(event, onActivity);
    };
  }, [wake]);

  useInterval(() => {
    if (!idle && Date.now() - lastActivity.current > idleTimer.current) {
      setIdle(true);
    }
  }, 1_000);

  /* Data ------------------------------------------------------------------- */

  const pushChat = useCallback((role: 'user' | 'assistant', text: string): void => {
    const trimmed = text.trim();
    if (trimmed === '') return;
    chatSequence.current += 1;
    setChatLog((current) => [
      ...current.slice(-3),
      { id: chatSequence.current, role, text: trimmed },
    ]);
    // Bubbles are ephemeral: they fade away unless the panel stays active.
    window.clearTimeout(chatTimer.current);
    chatTimer.current = window.setTimeout(() => setChatLog([]), 12_000);
  }, []);

  const speakReply = useCallback(
    (text: string): void => {
      const trimmed = text.trim();
      if (trimmed === '') return;
      const now = Date.now();
      if (lastSpoken.current.text === trimmed && now - lastSpoken.current.at < 4_000) {
        return;
      }
      lastSpoken.current = { text: trimmed, at: now };
      const fallback = (): void => {
        if (typeof window === 'undefined' || !('speechSynthesis' in window)) return;
        const synth = window.speechSynthesis;
        synth.cancel();
        const utterance = new SpeechSynthesisUtterance(trimmed);
        const voices = synth.getVoices();
        const preferred =
          voices.find((voice) =>
            voice.lang.toLowerCase().startsWith(language === 'en' ? 'en' : 'es'),
          ) ?? voices.find((voice) => voice.lang.toLowerCase().startsWith('es'));
        if (preferred !== undefined) utterance.voice = preferred;
        utterance.lang = preferred?.lang ?? (language === 'en' ? 'en-US' : 'es-ES');
        utterance.rate = 1;
        utterance.pitch = 1;
        synth.speak(utterance);
      };
      speechAudio.current?.pause();
      if (speechUrl.current !== null) {
        URL.revokeObjectURL(speechUrl.current);
        speechUrl.current = null;
      }
      void synthesizeKioskSpeech(trimmed)
        .then((blob) => {
          const url = URL.createObjectURL(blob);
          speechUrl.current = url;
          const audio = new Audio(url);
          speechAudio.current = audio;
          audio.onended = () => {
            if (speechUrl.current === url) {
              URL.revokeObjectURL(url);
              speechUrl.current = null;
            }
          };
          audio.onerror = fallback;
          void audio.play().catch(fallback);
        })
        .catch(fallback);
    },
    [language],
  );

  useEffect(() => {
    if (typeof window === 'undefined' || !('speechSynthesis' in window)) return;
    const synth = window.speechSynthesis;
    const warm = (): void => {
      synth.getVoices();
    };
    warm();
    synth.addEventListener('voiceschanged', warm);
    const unlock = (): void => {
      const silent = new SpeechSynthesisUtterance(' ');
      silent.volume = 0;
      synth.speak(silent);
      window.removeEventListener('pointerdown', unlock);
    };
    window.addEventListener('pointerdown', unlock);
    return () => {
      window.removeEventListener('pointerdown', unlock);
      synth.removeEventListener('voiceschanged', warm);
    };
  }, []);

  const applySnapshot = useCallback((snapshot: KioskStateResponse) => {
    setLive({
      profile: snapshot.profile,
      music: snapshot.music,
      scenes: snapshot.scenes,
      network: snapshot.network,
      agentActivity: snapshot.agent_activity,
    });
    setVolume(snapshot.music.volume);
    setAssistantName(snapshot.assistant_name);
  }, []);

  const reloadState = useCallback(() => {
    if (isDemo) return;
    void kioskApi
      .state()
      .then((snapshot) => applySnapshot(snapshot))
      .catch(() => undefined);
  }, [isDemo, applySnapshot]);

  useEffect(() => {
    if (isDemo) return;
    reloadState();
    const timer = window.setInterval(() => {
      if (Date.now() - lastEventAt.current > 20_000) reloadState();
    }, 15_000);
    return () => window.clearInterval(timer);
  }, [isDemo, reloadState]);

  useEffect(() => {
    if (isDemo) return;
    void profilesApi
      .list()
      .then((list) => setProfiles(list))
      .catch(() => {
        /* the kiosk has no admin session; the switcher falls back to the live profile */
      });
  }, [isDemo]);

  /* WebSocket events -------------------------------------------------------- */

  useEffect(() => {
    if (isDemo) return;

    const handleEvent = (event: KioskEvent): void => {
      switch (event.type) {
        case 'avatar':
          setAvatarState(event.state);
          break;
        case 'tracking':
          setTracking(event.point);
          if (event.point !== null) setAvatarState((current) => (current === 'idle' ? 'tracking' : current));
          else setAvatarState((current) => (current === 'tracking' ? 'idle' : current));
          break;
        case 'profile_activated':
          setLive((current) => ({ ...current, profile: event.profile }));
          setAvatarState('profile_switch');
          window.setTimeout(() => setAvatarState('idle'), 1_400);
          setProfileSheetOpen(false);
          break;
        case 'music':
          setLive((current) => ({ ...current, music: event.state }));
          setVolume(event.state.volume);
          break;
        case 'agent':
          setLive((current) => ({ ...current, agentActivity: event.activity }));
          setAvatarState(activityToAvatar(event.activity));
          break;
        case 'network':
          setLive((current) => ({ ...current, network: event.status }));
          break;
        case 'voice':
          setAssistantName(event.assistant_name);
          break;
        case 'wake':
          setAssistantName(event.assistant_name);
          setAvatarState('listening');
          setLive((current) => ({ ...current, agentActivity: 'listening' }));
          pushToast({
            level: 'info',
            title: t('kiosk.listening'),
            message: t('kiosk.hint', { name: event.assistant_name }),
          });
          break;
        case 'speech':
          setAvatarState('speaking');
          pushChat('assistant', event.text);
          speakReply(event.text);
          break;
        case 'notification':
          pushToast({
            level: event.level,
            title: t('kiosk.notification'),
            message: event.message,
          });
          break;
        default:
          break;
      }
    };

    const socket = new ReconnectingSocket<KioskEvent>('/kiosk/events', {
      onStatus: setWsStatus,
      onOpen: () => {
        lastEventAt.current = Date.now();
        reloadState();
      },
      onMessage: (raw) => {
        if (!isKioskEvent(raw)) return;
        lastEventAt.current = Date.now();
        handleEvent(raw);
      },
    });
    socket.connect();
    return () => socket.close();
    // Setters and refs are stable; the toast push callback is stable too.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [isDemo, pushToast, t, reloadState]);

  /* Actions ----------------------------------------------------------------- */

  const toggleMusic = async (): Promise<void> => {
    if (busy !== null) return;
    wake();
    setBusy('music');
    try {
      if (live.music.playing) await kioskApi.musicPause();
      else await kioskApi.musicPlay();
      setLive((current) => ({ ...current, music: { ...current.music, playing: !current.music.playing } }));
    } catch (cause) {
      pushToast({ level: 'error', message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(null);
    }
  };

  const commitVolume = async (next: number): Promise<void> => {
    wake();
    try {
      await kioskApi.setVolume(next);
      setLive((current) => ({ ...current, music: { ...current.music, volume: next } }));
    } catch (cause) {
      setVolume(live.music.volume);
      pushToast({ level: 'error', message: t(errorMessageKey(cause)) });
    }
  };

  const activateScene = async (scene: KioskScene): Promise<void> => {
    if (busy !== null) return;
    wake();
    setBusy(`scene:${scene.entity_id}`);
    try {
      await kioskApi.activateScene(scene.entity_id);
      pushToast({ level: 'success', message: t('ha.toast.sceneActivated', { name: scene.name }) });
    } catch (cause) {
      pushToast({ level: 'error', message: t(errorMessageKey(cause)) });
    } finally {
      setBusy(null);
    }
  };

  const activateProfile = async (profile: Profile): Promise<void> => {
    wake();
    setBusy(`profile:${profile.id}`);
    try {
      if (!isDemo) await kioskApi.activateProfile(profile.id);
      setLive((current) => ({ ...current, profile }));
      setProfileSheetOpen(false);
      setAvatarState('profile_switch');
      window.setTimeout(() => setAvatarState('idle'), 1_400);
    } catch {
      /* ignore: the switcher stays open so the user can retry */
    } finally {
      setBusy(null);
    }
  };

  const openTalk = (): void => {
    wake();
    setTalkOpen(true);
    setAvatarState('listening');
    if (!isDemo) void kioskApi.setListening(true).catch(() => undefined);
    if (micState !== 'granted' && micState !== 'denied' && micState !== 'unsupported') {
      void acquireMicrophone().then((stream) =>
        stream?.getTracks().forEach((track) => track.stop()),
      );
    }
    window.setTimeout(() => queryInput.current?.focus(), 50);
  };

  const acquireMicrophone = async (): Promise<MediaStream | null> => {
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      setMicState('granted');
      setMicError(null);
      return stream;
    } catch (cause) {
      const name = (cause as { name?: string } | null)?.name ?? '';
      if (name === 'NotAllowedError' || name === 'SecurityError') {
        setMicState('denied');
        setMicError(t('kiosk.micDenied'));
      } else if (name === 'NotFoundError') {
        setMicError(t('kiosk.micNotFound'));
      } else {
        setMicError(t('kiosk.micError'));
      }
      return null;
    }
  };

  const stopContinuous = (): void => {
    continuousRef.current = false;
    setContinuous(false);
    streamRef.current?.getTracks().forEach((track) => track.stop());
    streamRef.current = null;
  };

  const closeTalk = (): void => {
    setTalkOpen(false);
    setListening(false);
    setAvatarState('idle');
  };

  const setListening = (active: boolean): void => {
    if (isDemo) return;
    void kioskApi.setListening(active).catch(() => undefined);
  };

  const sendQuery = async (override?: string): Promise<void> => {
    const text = (override ?? queryText).trim();
    if (text === '' || querying) return;
    setQuerying(true);
    setQueryText('');
    setAgentTrace([]);
    pushChat('user', text);
    setAvatarState('thinking');
    setLive((current) => ({ ...current, agentActivity: 'thinking' }));
    try {
      const history = conversation.slice(-20);
      const response = await kioskApi.query(text, history);
      setConversation((current) =>
        [
          ...current,
          { role: 'user' as const, content: text },
          { role: 'assistant' as const, content: response.reply },
        ].slice(-24),
      );
      setAgentTrace(response.tool_trace ?? []);
      pushChat('assistant', response.reply);
      speakReply(response.reply);
      setLive((current) => ({ ...current, agentActivity: 'speaking' }));
      setAvatarState('speaking');
      window.clearTimeout(speechTimer.current);
      const duration = Math.min(8_000, Math.max(2_000, 1_200 + response.reply.length * 45));
      speechTimer.current = window.setTimeout(() => {
        setAvatarState('idle');
        setLive((current) => ({ ...current, agentActivity: 'idle' }));
      }, duration);
    } catch (cause) {
      pushChat('assistant', t(errorMessageKey(cause)));
      setAvatarState('idle');
      setLive((current) => ({ ...current, agentActivity: 'error' }));
    } finally {
      setQuerying(false);
    }
  };

  const recordClip = (stream: MediaStream, milliseconds: number): Promise<Blob> =>
    new Promise((resolve, reject) => {
      const chunks: Blob[] = [];
      const preferred = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : undefined;
      let recorder: MediaRecorder;
      try {
        recorder = new MediaRecorder(
          stream,
          preferred === undefined ? undefined : { mimeType: preferred },
        );
      } catch (cause) {
        reject(cause);
        return;
      }
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunks.push(event.data);
      };
      recorder.onstop = () =>
        resolve(new Blob(chunks, { type: recorder.mimeType || 'audio/webm' }));
      recorder.start();
      window.setTimeout(() => {
        if (recorder.state === 'recording') recorder.stop();
      }, milliseconds);
    });

  const recordUntilSilence = (
    stream: MediaStream,
    maxMs = 8_000,
    silenceMs = 650,
    threshold = 0.012,
  ): Promise<Blob> =>
    new Promise((resolve, reject) => {
      const chunks: Blob[] = [];
      const preferred = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : undefined;
      let recorder: MediaRecorder;
      try {
        recorder = new MediaRecorder(
          stream,
          preferred === undefined ? undefined : { mimeType: preferred },
        );
      } catch (cause) {
        reject(cause);
        return;
      }
      const context = new AudioContext();
      const source = context.createMediaStreamSource(stream);
      const analyser = context.createAnalyser();
      analyser.fftSize = 2_048;
      source.connect(analyser);
      const samples = new Float32Array(analyser.fftSize);
      let finished = false;
      const cleanup = (): void => {
        try {
          source.disconnect();
        } catch {
          /* already disconnected */
        }
        void context.close().catch(() => undefined);
      };
      const finish = (): void => {
        if (finished) return;
        finished = true;
        cleanup();
        resolve(new Blob(chunks, { type: recorder.mimeType || 'audio/webm' }));
      };
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunks.push(event.data);
      };
      recorder.onstop = finish;
      recorder.onerror = () => {
        cleanup();
        reject(new Error('recording failed'));
      };
      recorder.start();
      void context.resume().catch(() => undefined);
      const started = performance.now();
      let lastVoice = started;
      let heard = false;
      let maxRms = 0;
      const tick = (): void => {
        if (finished) return;
        analyser.getFloatTimeDomainData(samples);
        let energy = 0;
        for (let index = 0; index < samples.length; index += 1) {
          energy += samples[index] * samples[index];
        }
        const rms = Math.sqrt(energy / samples.length);
        maxRms = Math.max(maxRms, rms);
        const now = performance.now();
        if (rms >= threshold) {
          heard = true;
          lastVoice = now;
        }
        const elapsed = now - started;
        const shouldStop =
          (heard && now - lastVoice >= silenceMs) ||
          elapsed >= maxMs ||
          (!heard && elapsed >= 4_000);
        if (shouldStop) {
          console.debug('[vad] stop', {
            elapsed: Math.round(elapsed),
            heard,
            maxRms: Number(maxRms.toFixed(4)),
            rms: Number(rms.toFixed(4)),
          });
          if (recorder.state === 'inactive') {
            finish();
          } else {
            recorder.stop();
          }
          return;
        }
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
    });

  const continuousLoop = async (stream: MediaStream): Promise<void> => {
    while (continuousRef.current) {
      try {
        const clip = await recordClip(stream, 1_500);
        if (!continuousRef.current) return;
        if (!(await detectKioskWake(clip, assistantName))) continue;
        const outcome = await kioskApi.wake(assistantName);
        if (!outcome.activated) continue;
        setAssistantName(outcome.assistant_name);
        setAvatarState('listening');
        const requestClip = await recordUntilSilence(stream);
        let query = (await transcribeKioskAudioFast(requestClip)).trim();
        if (query === '') {
          query = (await transcribeKioskAudio(requestClip)).trim();
        }
        if (query !== '') await sendQuery(query);
      } catch {
        await new Promise((resolve) => window.setTimeout(resolve, 1_000));
      }
    }
  };

  const toggleContinuous = async (): Promise<void> => {
    if (continuous) {
      stopContinuous();
      return;
    }
    if (
      typeof navigator === 'undefined' ||
      navigator.mediaDevices?.getUserMedia === undefined ||
      typeof MediaRecorder === 'undefined'
    ) {
      pushChat('assistant', t('kiosk.recordUnsupported'));
      return;
    }
    const stream = await acquireMicrophone();
    if (stream === null) {
      pushChat('assistant', t('kiosk.micNeeded'));
      return;
    }
    streamRef.current = stream;
    continuousRef.current = true;
    setContinuous(true);
    void continuousLoop(stream);
  };

  const autoStarted = useRef(false);

  useEffect(() => {
    if (autoStarted.current) return;
    if (micState !== 'granted') return;
    if (continuousRef.current) return;
    if (
      typeof navigator === 'undefined' ||
      navigator.mediaDevices?.getUserMedia === undefined ||
      typeof MediaRecorder === 'undefined'
    ) {
      return;
    }
    autoStarted.current = true;
    void toggleContinuous();
  }, [micState, toggleContinuous]);

  const toggleRecording = async (): Promise<void> => {
    if (recording) {
      recorderRef.current?.stop();
      return;
    }
    if (
      typeof navigator === 'undefined' ||
      navigator.mediaDevices?.getUserMedia === undefined ||
      typeof MediaRecorder === 'undefined'
    ) {
      pushChat('assistant', t('kiosk.recordUnsupported'));
      return;
    }
    try {
      const stream = await acquireMicrophone();
      if (stream === null) {
        pushChat('assistant', t('kiosk.micNeeded'));
        return;
      }
      const preferred = MediaRecorder.isTypeSupported('audio/webm;codecs=opus')
        ? 'audio/webm;codecs=opus'
        : undefined;
      const recorder = new MediaRecorder(
        stream,
        preferred === undefined ? undefined : { mimeType: preferred },
      );
      chunksRef.current = [];
      recorder.ondataavailable = (event) => {
        if (event.data.size > 0) chunksRef.current.push(event.data);
      };
      recorder.onstop = () => {
        stream.getTracks().forEach((track) => track.stop());
        setRecording(false);
        setAvatarState('idle');
        const blob = new Blob(chunksRef.current, {
          type: recorder.mimeType || 'audio/webm',
        });
        if (blob.size === 0) return;
        setTranscribing(true);
        void transcribeKioskAudioFast(blob)
          .then((fast) => (fast === '' ? transcribeKioskAudio(blob) : fast))
          .then((transcript) => {
            if (transcript.trim() === '') {
              pushChat('assistant', t('kiosk.recordEmpty'));
              return;
            }
            return sendQuery(transcript);
          })
          .catch(() => {
            pushChat('assistant', t('kiosk.recordError'));
          })
          .finally(() => {
            setTranscribing(false);
          });
      };
      recorderRef.current = recorder;
      chunksRef.current = [];
      setRecording(true);
      setAvatarState('listening');
      recorder.start();
      window.setTimeout(() => {
        if (recorder.state === 'recording') recorder.stop();
      }, 8_000);
    } catch {
      pushChat('assistant', t('kiosk.recordPermission'));
    }
  };

  useEffect(
    () => () => {
      window.clearTimeout(speechTimer.current);
    },
    [],
  );

  /* Derived ---------------------------------------------------------------- */

  const profile = live.profile ?? (isDemo ? (DEMO_PROFILES[0] ?? null) : null);
  const greeting =
    profile !== null ? t('kiosk.greeting', { name: profile.name }) : t('kiosk.greetingGuest');
  const switcherProfiles = useMemo(() => {
    if (profiles.length > 0) return profiles;
    return live.profile !== null ? [live.profile] : [];
  }, [profiles, live.profile]);
  const agentLabel = t(ACTIVITY_KEYS[live.agentActivity]);
  const isOnline = isDemo ? true : wsStatus === 'open';

  return (
    <div
      className={cn(
        'relative flex h-[100dvh] w-full flex-col overflow-hidden bg-night-900 text-ink-100',
        idle && 'cursor-none',
      )}
      onPointerDown={wake}
    >
      {/* Ambient background ------------------------------------------------- */}
      <div aria-hidden="true" className="pointer-events-none absolute inset-0 overflow-hidden">
        <div className="absolute -left-40 top-[-20%] h-[70vh] w-[70vh] rounded-full bg-mint-500/[0.07] blur-3xl" />
        <div className="absolute -right-52 top-[10%] h-[60vh] w-[60vh] rounded-full bg-mint-400/[0.05] blur-3xl" />
        <div className="absolute bottom-[-30%] left-1/3 h-[50vh] w-[50vh] rounded-full bg-amber-400/[0.03] blur-3xl" />
        <div className="absolute left-[15%] top-[22%] h-1 w-1 animate-[hs-float-slow_9s_ease-in-out_infinite] rounded-full bg-mint-300/50" />
        <div className="absolute right-[24%] top-[16%] h-1.5 w-1.5 animate-[hs-float-slow_12s_ease-in-out_infinite] rounded-full bg-mint-300/30" />
      </div>

      {/* Header / clock ------------------------------------------------------ */}
      <header className="relative z-10 grid grid-cols-[1fr_auto_1fr] items-start gap-4 px-6 pt-5 sm:px-10 sm:pt-7">
        <div className="min-w-0">
          <p className="section-label">{isDemo ? t('common.demoMode') : t('app.os')}</p>
          <p className="mt-1 truncate text-[13px] text-ink-400">
            {formatDate(now, language)}
          </p>
        </div>
        <div className="text-center">
          <p className="font-display text-[clamp(2.4rem,6vw,4.2rem)] font-medium leading-none tracking-tight text-ink-50 tabular">
            {formatTime(now, language, { hour: '2-digit', minute: '2-digit', hour12: false })}
          </p>
          <p className="mt-1 text-[11px] uppercase tracking-[0.25em] text-ink-500">
            {Intl.DateTimeFormat().resolvedOptions().timeZone}
          </p>
        </div>
        <div className="flex flex-col items-end gap-2">
          <div className="flex items-center gap-2">
            {isDemo ? (
              <StatusPill label={t('kiosk.demo')} tone="warning" />
            ) : (
              <StatusPill
                label={isOnline ? t('common.connected') : t('kiosk.ws.reconnecting')}
                tone={isOnline ? 'success' : 'warning'}
                pulse={isOnline}
              />
            )}
            <StatusPill
              label={
                live.network.online
                  ? t('kiosk.network.online')
                  : t('kiosk.network.offline')
              }
              tone={live.network.online ? 'success' : 'danger'}
            />
          </div>
          {!isDemo && wsStatus !== 'open' ? (
            <p className="text-[10px] text-ink-600">{t('kiosk.ws.reconnecting')}</p>
          ) : null}
        </div>
      </header>

      {/* Main ---------------------------------------------------------------- */}
      <main className="relative z-10 grid min-h-0 flex-1 grid-cols-1 gap-4 px-6 py-4 sm:px-10 lg:gap-8">
        {/* Avatar */}
        <section className="flex min-h-0 flex-col items-center justify-center gap-5">
          <button
            type="button"
            onClick={openTalk}
            className="group relative flex flex-col items-center gap-5 rounded-[36px] focus-visible:outline-none"
            aria-label={t('kiosk.talkHint')}
          >
            {(avatarState === 'listening' || live.agentActivity === 'listening') ? (
              <div className="listening-bars flex h-10 items-end gap-1.5" aria-hidden="true">
                {Array.from({ length: 9 }).map((_, index) => (
                  <span
                    key={index}
                    className="w-1.5 rounded-full bg-mint-400/70"
                    style={{
                      height: `${10 + ((index * 7) % 26)}px`,
                      animationDelay: `${index * 0.09}s`,
                    }}
                  />
                ))}
              </div>
            ) : (
              <span className="h-10" aria-hidden="true" />
            )}
            <BoopAvatar
              avatar={
                profile?.avatar ?? {
                  shape: 'squircle',
                  color: '#6ee7ff',
                  eye_color: '#0b1220',
                  eye_tilt_deg: 0,
                }
              }
              state={avatarState}
              size={avatarSize}
              tracking={tracking}
            />
          </button>
          <div className="text-center">
            <h2 className="font-display text-[clamp(1.8rem,3.4vw,2.6rem)] font-medium tracking-tight text-ink-50">
              {greeting}
            </h2>
            <p className="mt-1 text-[13px] text-ink-400">
              {avatarState === 'listening'
                ? t('kiosk.listening')
                : avatarState === 'thinking'
                  ? t('kiosk.thinking')
                  : avatarState === 'speaking'
                    ? t('kiosk.speaking')
                    : t('kiosk.hint', { name: assistantName })}
            </p>
          </div>
        </section>

        {/* Compact dock: weather + domotics + talk. Details open on demand. */}
        <div className="fixed right-4 top-24 z-30 flex w-[188px] flex-col gap-2.5 sm:right-6 lg:right-8">
          <div className="glass rounded-2xl p-3">
            <div className="flex items-center gap-2 text-ink-400">
              <CloudSun size={15} className="text-amber-300" />
              <span className="text-[10px] font-semibold uppercase tracking-[0.14em]">
                {t('kiosk.weather')} · Lima
              </span>
            </div>
            <p className="mt-1.5 font-display text-[26px] font-medium tabular text-ink-50">
              {weather === null ? '—°' : `${Math.round(weather.temperature_c)}°`}
            </p>
            <p className="text-[10.5px] leading-snug text-ink-400">
              {weather === null ? t('kiosk.weatherUnavailable') : weather.description}
            </p>
            {weather !== null ? (
              <p className="mt-1 text-[10px] text-ink-500">
                {t('kiosk.weatherHumidity', { value: Math.round(weather.humidity) })}
              </p>
            ) : null}
          </div>
          <button
            type="button"
            onClick={() => setAutomationOpen(true)}
            className="glass flex items-center justify-between gap-2 rounded-2xl px-3.5 py-3 text-left text-[12px] text-ink-200 transition hover:border-mint-400/30 hover:text-mint-100"
          >
            <span className="flex items-center gap-2">
              <Home size={14} className="text-amber-300" />
              {t('kiosk.automation')}
            </span>
            <span className="text-[10px] text-ink-500">
              {live.scenes.length + (live.music.playing ? 1 : 0)}
            </span>
          </button>
          <button
            type="button"
            onClick={openTalk}
            className="flex items-center justify-center gap-2 rounded-2xl border border-mint-400/25 bg-mint-400/[0.08] px-3 py-3 text-[12px] font-medium text-mint-100 transition hover:bg-mint-400/[0.13]"
          >
            <Mic size={15} />
            {t('kiosk.talk')}
          </button>
        </div>

        {automationOpen ? (
          <aside className="fixed bottom-24 right-4 top-28 z-40 w-[min(92vw,380px)] overflow-y-auto rounded-[28px] border border-white/10 bg-[#08111d]/95 p-4 shadow-2xl backdrop-blur-xl">
            <div className="mb-3 flex items-start justify-between gap-3">
              <div>
                <p className="text-[11px] font-semibold uppercase tracking-[0.14em] text-ink-300">
                  {t('kiosk.automation')}
                </p>
                <p className="mt-0.5 text-[10.5px] text-ink-500">{t('kiosk.automationHint')}</p>
              </div>
              <button
                type="button"
                onClick={() => setAutomationOpen(false)}
                className="grid h-8 w-8 place-items-center rounded-lg text-ink-400 transition hover:bg-white/[0.06] hover:text-ink-100"
                aria-label={t('kiosk.close')}
              >
                <X size={16} />
              </button>
            </div>
            <div className="flex flex-col gap-3">
          {/* Music */}
          <section className="glass p-4 sm:p-5">
            <div className="flex items-center justify-between gap-2">
              <div className="flex items-center gap-2 text-ink-400">
                <Music2 size={15} className="text-mint-400" />
                <span className="text-[11px] font-semibold uppercase tracking-[0.14em]">
                  {t('kiosk.music.title')}
                </span>
              </div>
              <StatusPill
                label={live.music.playing ? t('kiosk.music.nowPlaying') : t('kiosk.music.paused')}
                tone={live.music.playing ? 'success' : 'neutral'}
              />
            </div>
            <div className="mt-3 flex items-center gap-4">
              <button
                type="button"
                onClick={() => void toggleMusic()}
                className="grid h-14 w-14 shrink-0 place-items-center rounded-2xl bg-mint-gradient text-[#04121a] shadow-[0_12px_30px_-14px_rgba(110,231,255,0.8)] transition hover:brightness-110"
                aria-label={live.music.playing ? t('kiosk.music.pause') : t('kiosk.music.play')}
                disabled={busy === 'music'}
              >
                {busy === 'music' ? (
                  <Spinner size={20} className="text-[#04121a]" />
                ) : live.music.playing ? (
                  <Pause size={22} />
                ) : (
                  <Play size={22} className="ml-0.5" />
                )}
              </button>
              <div className="min-w-0 flex-1">
                <p className="truncate text-[14px] font-medium text-ink-50">
                  {live.music.track ?? t('kiosk.music.empty')}
                </p>
                <p className="truncate text-[11.5px] text-ink-400">
                  {live.music.artist ?? t('kiosk.music.unknownArtist')}
                </p>
                {live.music.sink !== null ? (
                  <p className="mt-0.5 truncate text-[10px] text-ink-600">
                    {t('kiosk.music.sink', { name: live.music.sink })}
                  </p>
                ) : null}
              </div>
            </div>
            <div className="mt-3 flex items-center gap-2.5">
              <Volume2 size={14} className="shrink-0 text-ink-500" />
              <input
                type="range"
                min="0"
                max="100"
                value={volume}
                onChange={(event) => setVolume(Number(event.target.value))}
                onPointerUp={() => void commitVolume(volume)}
                onKeyUp={() => void commitVolume(volume)}
                aria-label={t('kiosk.music.volume')}
                className="h-14 flex-1 cursor-pointer accent-[#6ee7ff]"
              />
              <span className="w-9 text-right text-[11px] tabular text-ink-500">{volume}</span>
            </div>
          </section>

          {/* Scenes */}
          <section className="glass p-4 sm:p-5">
            <div className="flex items-center gap-2 text-ink-400">
              <Sparkles size={15} className="text-amber-300" />
              <span className="text-[11px] font-semibold uppercase tracking-[0.14em]">
                {t('kiosk.scenes.title')}
              </span>
            </div>
            {live.scenes.length === 0 ? (
              <p className="mt-3 text-[12px] text-ink-500">{t('kiosk.scenes.empty')}</p>
            ) : (
              <div className="mt-3 grid grid-cols-2 gap-2">
                {live.scenes.map((scene) => (
                  <button
                    key={scene.entity_id}
                    type="button"
                    onClick={() => void activateScene(scene)}
                    disabled={busy !== null}
                    className="min-h-[56px] rounded-xl border border-white/[0.07] bg-white/[0.03] px-3 py-2 text-left text-[12.5px] text-ink-200 transition hover:border-amber-300/30 hover:bg-amber-300/[0.06] hover:text-amber-100 disabled:opacity-60"
                  >
                    <span className="flex items-center gap-2">
                      {busy === `scene:${scene.entity_id}` ? (
                        <Spinner size={13} />
                      ) : (
                        <Home size={13} className="text-amber-300/80" />
                      )}
                      <span className="truncate">{scene.name}</span>
                    </span>
                  </button>
                ))}
              </div>
            )}
          </section>

          {/* Network */}
          <section className="glass p-4 sm:p-5">
            <div className="flex items-center gap-2 text-ink-400">
              <Wifi size={15} className="text-mint-400" />
              <span className="text-[11px] font-semibold uppercase tracking-[0.14em]">
                {t('kiosk.network.title')}
              </span>
            </div>
            <div className="mt-3 space-y-2">
              <NetworkRow
                icon={<Wifi size={13} />}
                label={live.network.online ? t('kiosk.network.online') : t('kiosk.network.offline')}
                ok={live.network.online}
              />
              <NetworkRow
                icon={<ShieldCheck size={13} />}
                label={live.network.dns_ok ? t('kiosk.network.dnsOk') : t('kiosk.network.dnsFail')}
                ok={live.network.dns_ok}
              />
              <div className="flex items-center justify-between text-[11.5px] text-ink-400">
                <span className="text-ink-500">{t('kiosk.network.latencyLabel')}</span>
                <span className="tabular text-mint-300">
                  {live.network.latency_ms !== null
                    ? t('kiosk.network.latency', { ms: live.network.latency_ms })
                    : '—'}
                </span>
              </div>
            </div>
          </section>

            </div>
          </aside>
        ) : null}
      </main>

      {/* Footer / agent status ----------------------------------------------- */}
      <footer className="relative z-10 flex items-center justify-between gap-4 border-t border-white/[0.05] px-6 py-3.5 sm:px-10">
        <div className="flex min-w-0 items-center gap-3">
          <span
            aria-hidden="true"
            className={cn(
              'h-2 w-2 shrink-0 rounded-full',
              live.agentActivity === 'error'
                ? 'bg-coral-400'
                : live.agentActivity === 'idle'
                  ? 'bg-mint-500'
                  : 'bg-amber-400 animate-[hs-pulse-dot_1.6s_ease-in-out_infinite]',
            )}
          />
          <span className="flex items-center gap-2 truncate text-[12px] text-ink-400">
            <AudioLines size={14} className="text-ink-500" />
            {agentLabel}
          </span>
          {!isDemo && wsStatus !== 'open' ? (
            <span className="hidden items-center gap-1.5 text-[11px] text-amber-300/80 sm:flex">
              <CloudOff size={12} />
              {t('kiosk.ws.offline')}
            </span>
          ) : null}
          {isDemo ? (
            <span className="hidden text-[11px] text-ink-600 sm:inline">{t('kiosk.demoNotice')}</span>
          ) : null}
        </div>

        <div className="flex items-center gap-2">
          {isDemo ? (
            <div className="hidden items-center gap-1 lg:flex">
              {DEMO_STATES.map((state) => (
                <button
                  key={state}
                  type="button"
                  onClick={() => setAvatarState(state)}
                  aria-pressed={avatarState === state}
                  className={cn(
                    'min-h-[30px] rounded-full border px-2 text-[10px] transition',
                    avatarState === state
                      ? 'border-mint-400/35 bg-mint-400/[0.08] text-mint-200'
                      : 'border-white/[0.07] text-ink-500 hover:text-ink-300',
                  )}
                >
                  {t(`avatar.state.${state}` as DictKey)}
                </button>
              ))}
            </div>
          ) : null}
          <button
            type="button"
            onClick={() => {
              wake();
              setProfileSheetOpen(true);
            }}
            className="flex min-h-[56px] items-center gap-2.5 rounded-2xl border border-white/[0.08] bg-white/[0.03] px-3 py-1.5 transition hover:border-white/[0.16]"
            aria-label={t('kiosk.profiles.switch')}
          >
            {profile !== null ? (
              <BoopAvatar avatar={profile.avatar} state="idle" size={34} decorative />
            ) : (
              <span className="grid h-8 w-8 place-items-center rounded-full border border-white/10 text-ink-500">
                <Mic size={14} />
              </span>
            )}
            <span className="text-left">
              <span className="block max-w-[110px] truncate text-[12px] font-medium text-ink-100">
                {profile?.name ?? t('kiosk.profileMissing')}
              </span>
              <span className="block text-[10px] uppercase tracking-wider text-ink-500">
                {t('kiosk.profiles.switch')}
              </span>
            </span>
          </button>
        </div>
      </footer>

      {/* Idle overlay -------------------------------------------------------- */}
      <div
        aria-hidden="true"
        className={cn(
          'pointer-events-none absolute inset-0 z-20 bg-night-950 transition-opacity duration-1000',
          idle ? 'opacity-45' : 'opacity-0',
        )}
      />
      {idle ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-24 z-30 flex justify-center">
          <span className="rounded-full border border-white/[0.08] bg-night-900/70 px-4 py-2 text-[12px] text-ink-400 backdrop-blur-md">
            {t('kiosk.pressToWake', { name: assistantName })}
          </span>
        </div>
      ) : null}

      {/* Profile switcher ---------------------------------------------------- */}
      {profileSheetOpen ? (
        <div className="fixed inset-0 z-50 flex items-end justify-center sm:items-center">
          <div
            aria-hidden="true"
            className="absolute inset-0 bg-night-950/70 backdrop-blur-sm"
            onClick={() => setProfileSheetOpen(false)}
          />
          <div className="glass-strong relative z-10 m-4 w-full max-w-lg rounded-[26px] p-5 animate-[hs-scale-in_200ms_var(--hs-ease)]">
            <div className="flex items-center justify-between">
              <h2 className="font-display text-[18px] font-medium text-ink-50">
                {t('kiosk.profiles.title')}
              </h2>
              <button
                type="button"
                onClick={() => setProfileSheetOpen(false)}
                className="grid h-10 w-10 place-items-center rounded-xl text-ink-400 transition hover:bg-white/[0.06] hover:text-ink-100"
                aria-label={t('kiosk.close')}
              >
                <X size={18} />
              </button>
            </div>
            <div className="mt-4 space-y-2">
              {switcherProfiles.length === 0 ? (
                <p className="text-[12.5px] text-ink-500">{t('kiosk.profileMissing')}</p>
              ) : (
                switcherProfiles.map((item) => {
                  const active = live.profile?.id === item.id;
                  return (
                    <button
                      key={item.id}
                      type="button"
                      onClick={() => void activateProfile(item)}
                      disabled={busy !== null}
                      className={cn(
                        'flex min-h-[72px] w-full items-center gap-3 rounded-2xl border px-3.5 py-2.5 text-left transition',
                        active
                          ? 'border-mint-400/30 bg-mint-400/[0.07]'
                          : 'border-white/[0.07] bg-white/[0.02] hover:border-white/[0.16]',
                      )}
                    >
                      <BoopAvatar avatar={item.avatar} state="idle" size={56} decorative />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[14px] font-medium text-ink-50">{item.name}</span>
                        <span className="block text-[11px] text-ink-500">
                          {t(`prof.role.${item.role}` as DictKey)}
                        </span>
                      </span>
                      {busy === `profile:${item.id}` ? <Spinner size={16} /> : null}
                      {active ? <StatusPill label={t('kiosk.profileSwitchHint')} tone="success" /> : null}
                    </button>
                  );
                })
              )}
            </div>
          </div>
        </div>
      ) : null}

      {/* Left chat panel: appears while writing, bubbles are ephemeral. ----- */}
      {talkOpen || chatLog.length > 0 || querying ? (
        <aside className="fixed bottom-20 left-3 z-50 w-[min(92vw,400px)] sm:left-5">
          <div className="glass-strong rounded-[24px] p-4 shadow-glow animate-[hs-fade-up_220ms_var(--hs-ease)]">
            <div className="flex items-center justify-between gap-3">
              <div className="flex items-center gap-2">
                <span className="grid h-8 w-8 place-items-center rounded-full border border-mint-400/25 bg-mint-400/[0.09] text-mint-300">
                  <Mic size={14} />
                </span>
                <div>
                  <p className="text-[12.5px] font-medium text-ink-50">{t('kiosk.talk')}</p>
                  <p className="text-[10px] text-ink-500">{t('kiosk.hint', { name: assistantName })}</p>
                </div>
              </div>
              <div className="flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => void toggleContinuous()}
                  className={cn(
                    'grid h-8 w-8 place-items-center rounded-lg transition',
                    continuous
                      ? 'bg-mint-400/[0.16] text-mint-300'
                      : 'text-ink-400 hover:bg-white/[0.06] hover:text-ink-100',
                  )}
                  title={continuous ? t('kiosk.continuousOn') : t('kiosk.continuous')}
                  aria-label={continuous ? t('kiosk.continuousOn') : t('kiosk.continuous')}
                >
                  <AudioLines size={16} />
                </button>
                <button
                  type="button"
                  onClick={closeTalk}
                  className="grid h-8 w-8 place-items-center rounded-lg text-ink-400 transition hover:bg-white/[0.06] hover:text-ink-100"
                  aria-label={t('kiosk.close')}
                >
                  <X size={16} />
                </button>
              </div>
            </div>

            {chatLog.length > 0 ? (
              <div className="mt-3 max-h-[38vh] space-y-2 overflow-y-auto pr-0.5">
                {chatLog.map((message) => (
                  <div
                    key={message.id}
                    className={cn(
                      'flex',
                      message.role === 'user' ? 'justify-end' : 'justify-start',
                    )}
                  >
                    <p
                      className={cn(
                        'max-w-[88%] whitespace-pre-wrap rounded-2xl px-3.5 py-2 text-[12.5px] leading-relaxed',
                        message.role === 'user'
                          ? 'bg-mint-400/[0.13] text-mint-100'
                          : 'border border-white/[0.07] bg-white/[0.03] text-ink-100',
                      )}
                    >
                      {message.text}
                    </p>
                  </div>
                ))}
              </div>
            ) : null}

            {talkOpen && micState !== 'granted' ? (
              <div className="mt-3 rounded-xl border border-amber-400/25 bg-amber-400/[0.06] px-3 py-2 text-[11px] leading-relaxed text-amber-200">
                <p>{micError ?? t('kiosk.micNeeded')}</p>
                {micState === 'denied' ? (
                  <p className="mt-1 text-amber-200/80">{t('kiosk.micDeniedHint')}</p>
                ) : null}
                {micState === 'unsupported' ? (
                  <p className="mt-1 text-amber-200/80">{t('kiosk.micUnsupported')}</p>
                ) : null}
                <button
                  type="button"
                  className="mt-1.5 rounded-lg border border-amber-300/40 px-2.5 py-1 text-[11px] font-medium text-amber-100 transition hover:bg-amber-300/10"
                  onClick={() =>
                    void acquireMicrophone().then((stream) =>
                      stream?.getTracks().forEach((track) => track.stop()),
                    )
                  }
                >
                  {micState === 'denied' ? t('kiosk.micRetry') : t('kiosk.enableMic')}
                </button>
              </div>
            ) : null}

            {agentTrace.length > 0 ? (
              <details className="mt-2 rounded-xl border border-white/[0.07] bg-white/[0.03] px-2.5 py-2 text-[10px] text-ink-400">
                <summary className="cursor-pointer select-none">
                  {t('kiosk.agentTrace', { count: agentTrace.length })}
                </summary>
                <ul className="mt-1.5 space-y-1">
                  {agentTrace.map((entry, index) => (
                    <li key={`${entry.tool}-${index}`} className="flex items-center justify-between gap-2">
                      <span className="truncate font-mono text-[9.5px] text-ink-300">{entry.tool}</span>
                      <span className={entry.error === null ? 'text-mint-300/70' : 'text-coral-300'}>
                        {entry.error === null ? `${entry.duration_ms} ms` : 'error'}
                      </span>
                    </li>
                  ))}
                </ul>
              </details>
            ) : null}

            {talkOpen ? (
              <>
                <button
                  type="button"
                  onClick={() => void toggleContinuous()}
                  className={cn(
                    'mt-3 inline-flex min-h-[34px] items-center gap-1.5 rounded-xl border px-3 text-[11.5px] transition',
                    continuous
                      ? 'border-mint-400/35 bg-mint-400/[0.10] text-mint-200'
                      : 'border-white/[0.10] bg-white/[0.03] text-ink-300 hover:border-white/[0.18]',
                  )}
                >
                  <AudioLines size={14} />
                  {continuous
                    ? t('kiosk.continuousOn')
                    : t('kiosk.startContinuous', { name: assistantName })}
                </button>
                <p className="mt-3 text-[11px] text-ink-400">
                  {querying
                    ? t('kiosk.thinking')
                    : avatarState === 'listening'
                      ? t('kiosk.listening')
                      : t('kiosk.talkHint')}
                </p>
                {micState === 'granted' && !continuous ? (
                  <p className="mt-1 text-[10.5px] text-mint-300/70">
                    {t('kiosk.micReadyHint', { name: assistantName })}
                  </p>
                ) : null}
                <form
                  className="mt-2 flex items-center gap-2"
                  onSubmit={(event) => {
                    event.preventDefault();
                    void sendQuery();
                  }}
                >
                  <input
                    ref={queryInput}
                    value={queryText}
                    onChange={(event) => setQueryText(event.target.value)}
                    placeholder={t('kiosk.queryPlaceholder')}
                    aria-label={t('kiosk.queryPlaceholder')}
                    disabled={querying || transcribing}
                    className="input min-h-[46px] flex-1"
                  />
                  <button
                    type="button"
                    className={cn(
                      'btn min-h-[46px] px-3',
                      recording ? 'btn-primary animate-pulse' : 'btn-secondary',
                    )}
                    onClick={() => void toggleRecording()}
                    disabled={querying || transcribing}
                    title={t('kiosk.recordHint')}
                    aria-label={recording ? t('kiosk.stopRecording') : t('kiosk.record')}
                  >
                    {transcribing ? (
                      <Spinner size={16} />
                    ) : recording ? (
                      <Square size={16} />
                    ) : (
                      <Mic size={16} />
                    )}
                  </button>
                  <button
                    type="submit"
                    className="btn btn-primary min-h-[46px] px-4"
                    disabled={querying || transcribing || queryText.trim() === ''}
                  >
                    {querying ? <Spinner size={16} className="text-[#04121a]" /> : null}
                    {t('agent.chat.send')}
                  </button>
                </form>
                {continuous ? (
                  <p className="mt-1.5 text-[10.5px] text-mint-300/80">
                    {t('kiosk.continuousActive', { name: assistantName })}
                  </p>
                ) : recording || transcribing ? (
                  <p className="mt-1.5 text-[10.5px] text-mint-300/80">
                    {transcribing ? t('kiosk.transcribing') : t('kiosk.recording')}
                  </p>
                ) : null}
              </>
            ) : null}
          </div>
        </aside>
      ) : null}

      {/* Offline retry ------------------------------------------------------- */}
      {!isDemo && live.profile === null && wsStatus !== 'open' ? (
        <div className="pointer-events-none absolute inset-x-0 top-24 z-30 flex justify-center">
          <div className="pointer-events-auto flex items-center gap-3 rounded-2xl border border-amber-400/25 bg-amber-400/[0.08] px-4 py-2.5 backdrop-blur-md">
            <CloudOff size={15} className="text-amber-300" />
            <span className="text-[12px] text-amber-200">{t('kiosk.ws.offline')}</span>
            <button
              type="button"
              className="inline-flex min-h-[32px] items-center gap-1 rounded-lg border border-amber-400/25 px-2.5 text-[11px] text-amber-200 transition hover:bg-amber-400/10"
              onClick={reloadState}
            >
              <RefreshCw size={12} />
              {t('common.retry')}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

function NetworkRow({
  icon,
  label,
  ok,
}: {
  icon: ReactNode;
  label: string;
  ok: boolean;
}): React.JSX.Element {
  return (
    <div className="flex items-center justify-between text-[11.5px]">
      <span className="flex items-center gap-2 text-ink-400">
        <span className={ok ? 'text-mint-400' : 'text-coral-400'}>{icon}</span>
        {label}
      </span>
      <span
        aria-hidden="true"
        className={cn('h-1.5 w-1.5 rounded-full', ok ? 'bg-mint-500' : 'bg-coral-400')}
      />
    </div>
  );
}

function activityToAvatar(activity: AgentActivity): AvatarState {
  switch (activity) {
    case 'listening':
      return 'listening';
    case 'thinking':
      return 'thinking';
    case 'speaking':
      return 'speaking';
    default:
      return 'idle';
  }
}







