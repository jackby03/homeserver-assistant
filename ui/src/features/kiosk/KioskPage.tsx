import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useSearchParams } from 'react-router-dom';
import {
  AudioLines,
  CloudOff,
  Home,
  Mic,
  Music2,
  Pause,
  Play,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Volume2,
  Wifi,
  X,
} from 'lucide-react';
import { kioskApi } from '@/api/kiosk';
import { profilesApi } from '@/api/profiles';
import { isKioskEvent } from '@/api/types';
import type {
  AgentActivity,
  AvatarState,
  KioskEvent,
  KioskScene,
  KioskStateResponse,
  MusicState,
  NetworkHealth,
  Profile,
  TrackingPoint,
} from '@/api/types';
import { ReconnectingSocket } from '@/api/ws';
import type { SocketStatus } from '@/api/ws';
import { AvatarFigure } from '@/components/Avatar';
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

  const initial = isDemo ? DEMO_KIOSK_STATE : null;
  const [live, setLive] = useState<LiveState>({
    profile: initial?.profile ?? null,
    music: initial?.music ?? EMPTY_MUSIC,
    scenes: initial?.scenes ?? [],
    network: initial?.network ?? EMPTY_NETWORK,
    agentActivity: initial?.agent_activity ?? 'idle',
  });
  const [assistantName, setAssistantName] = useState(initial?.assistant_name ?? 'Aura');
  const [listenerConnected, setListenerConnected] = useState(
    initial?.voice.listener_connected ?? false,
  );
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
  const chatTimer = useRef(0);
  const chatSequence = useRef(0);
  const queryInput = useRef<HTMLInputElement | null>(null);
  const [querying, setQuerying] = useState(false);
  const [idle, setIdle] = useState(false);
  const idleTimer = useRef<number>(0);
  const lastActivity = useRef(Date.now());
  const lastEventAt = useRef(0);
  const speechTimer = useRef(0);

  /* Clock / viewport / idle ------------------------------------------------- */

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
      if (typeof window === 'undefined' || !('speechSynthesis' in window)) return;
      const synth = window.speechSynthesis;
      synth.cancel();
      const utterance = new SpeechSynthesisUtterance(trimmed);
      const voices = synth.getVoices();
      const preferred =
        voices.find((voice) => voice.lang.toLowerCase().startsWith(language === 'en' ? 'en' : 'es')) ??
        voices.find((voice) => voice.lang.toLowerCase().startsWith('es'));
      if (preferred !== undefined) utterance.voice = preferred;
      utterance.lang = preferred?.lang ?? (language === 'en' ? 'en-US' : 'es-ES');
      utterance.rate = 1;
      utterance.pitch = 1;
      synth.speak(utterance);
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
    setListenerConnected(snapshot.voice.listener_connected);
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
          if (!talkOpen) speakReply(event.text);
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
  }, [isDemo, pushToast, t]);

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
    window.setTimeout(() => queryInput.current?.focus(), 50);
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

  const sendQuery = async (): Promise<void> => {
    const text = queryText.trim();
    if (text === '' || querying) return;
    setQuerying(true);
    setQueryText('');
    pushChat('user', text);
    setAvatarState('thinking');
    setLive((current) => ({ ...current, agentActivity: 'thinking' }));
    try {
      const response = await kioskApi.query(text);
      pushChat('assistant', response.reply);
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
      <main className="relative z-10 grid min-h-0 flex-1 grid-cols-1 gap-4 px-6 py-4 sm:px-10 lg:grid-cols-[minmax(0,1fr)_minmax(320px,380px)] lg:gap-8">
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
            <AvatarFigure
              avatar={
                profile?.avatar ?? {
                  shape: 'squircle',
                  color: '#6ee7ff',
                  eye_color: '#0b1220',
                  eye_tilt_deg: 0,
                }
              }
              state={avatarState}
              tracking={tracking}
              size={avatarSize}
              className="transition-transform duration-500 group-hover:scale-[1.01]"
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
            {!listenerConnected && !isDemo ? (
              <p className="mt-2 text-[11px] text-amber-300/80">
                {t('kiosk.listenerOffline')}
              </p>
            ) : null}
          </div>
        </section>

        {/* Quick tiles */}
        <aside className="flex min-h-0 flex-col gap-3 overflow-y-auto pb-1 lg:max-h-[calc(100dvh-14rem)]">
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

          {/* Talk */}
          <button
            type="button"
            onClick={openTalk}
            className="group flex min-h-[76px] items-center justify-center gap-3 rounded-[22px] border border-mint-400/25 bg-mint-400/[0.07] text-[15px] font-medium text-mint-100 transition hover:border-mint-400/40 hover:bg-mint-400/[0.11] active:scale-[0.995]"
          >
            <span className="grid h-11 w-11 place-items-center rounded-full bg-mint-gradient text-[#04121a] shadow-[0_10px_26px_-12px_rgba(110,231,255,0.8)]">
              <Mic size={20} />
            </span>
            {t('kiosk.talk')}
          </button>
        </aside>
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
              <AvatarFigure avatar={profile.avatar} state="idle" size={34} decorative />
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
                      <AvatarFigure avatar={item.avatar} state="idle" size={56} decorative />
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
              <button
                type="button"
                onClick={closeTalk}
                className="grid h-8 w-8 place-items-center rounded-lg text-ink-400 transition hover:bg-white/[0.06] hover:text-ink-100"
                aria-label={t('kiosk.close')}
              >
                <X size={16} />
              </button>
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

            {talkOpen ? (
              <>
                <p className="mt-3 text-[11px] text-ink-400">
                  {querying
                    ? t('kiosk.thinking')
                    : avatarState === 'listening'
                      ? t('kiosk.listening')
                      : t('kiosk.talkHint')}
                </p>
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
                    disabled={querying}
                    className="input min-h-[46px] flex-1"
                  />
                  <button
                    type="submit"
                    className="btn btn-primary min-h-[46px] px-4"
                    disabled={querying || queryText.trim() === ''}
                  >
                    {querying ? <Spinner size={16} className="text-[#04121a]" /> : <Mic size={16} />}
                    {t('agent.chat.send')}
                  </button>
                </form>
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


