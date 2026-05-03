import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import {
  Bug,
  ChatCenteredDots,
  Fish,
  GameController,
  Gear,
  Moon,
  NavigationArrow,
  Plug,
  SpinnerGap,
  Waves,
} from "@phosphor-icons/react"

import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Textarea } from "@/components/ui/textarea"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import { Badge } from "@/components/ui/badge"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { MinimapPanel } from "@/components/minimap"
import { TileSprite } from "@/components/TileSprite"
import {
  automateTutorial,
  connectWithAuth,
  disconnectSession,
  getDashboardAuthStatus,
  getMinimap,
  getAuthToken,
  joinWorld,
  leaveWorld,
  loginDashboard,
  listSessions,
  loadBlockTypes,
  logoutDashboard,
  moveSession,
  registerDashboardPassword,
  setAuthToken,
  getLuaScriptStatus,
  reconnectSession,
  startFishing,
  startLuaScript,
  startSpam,
  stopLuaScript,
  stopFishing,
  stopSpam,
  talk,
  wearItem,
  dropItem,
  type ActionResponse,
} from "@/lib/api"
import {
  buildWebSocketUrl,
  camelToWords,
  generateDeviceId,
  getItemCategory,
  inventoryTypeLabel,
  sortSessions,
  statusVariant,
} from "@/lib/dashboard"
import type {
  AuthInput,
  AuthKind,
  BlockNameMap,
  DashboardAuthStatus,
  LogEvent,
  LuaScriptStatusSnapshot,
  MinimapSnapshot,
  ServerEvent,
  SessionSnapshot,
  TutorialCompletedEvent,
} from "@/lib/types"

type Feedback = {
  kind: "success" | "error"
  message: string
}

type SessionInputs = {
  world: string
  bait: string
  chat: string
  luaSource: string
  spam: string
  spamDelay: string
}

const EMPTY_INPUTS: SessionInputs = {
  world: "",
  bait: "",
  chat: "",
  luaSource: "",
  spam: "",
  spamDelay: "5",
}

const CLOTHING_TYPES = new Set([768, 1024])
const LOG_TOKEN_PATTERN =
  /([A-Z][A-Z0-9]{0,10})(?= \{)|([A-Za-z_][A-Za-z0-9_]*=)|("(?:[^"\\]|\\.)*")|(\b\d+(?:\.\d+)?\b)/g

function formatLogTimestamp(timestampMs: number) {
  return new Date(timestampMs).toISOString().replace("T", " ").replace("Z", " UTC")
}

function logPrefixLabel(event: LogEvent) {
  if (event.direction === "incoming") {
    return "[<]"
  }
  if (event.direction === "outgoing") {
    return "[>]"
  }
  if (event.level === "warn") {
    return "[!]"
  }
  if (event.level === "error") {
    return "[x]"
  }
  if (event.level === "state") {
    return "[*]"
  }
  return "[i]"
}

function logPrefixClass(event: LogEvent) {
  if (event.direction === "incoming") {
    return "text-emerald-300"
  }
  if (event.direction === "outgoing") {
    return "text-sky-300"
  }
  if (event.level === "warn") {
    return "text-amber-300"
  }
  if (event.level === "error") {
    return "text-rose-300"
  }
  if (event.level === "state") {
    return "text-fuchsia-300"
  }
  return "text-slate-300"
}

function logScopeClass(event: LogEvent) {
  if (event.transport === "tcp") {
    return "border-sky-500/30 bg-sky-500/10 text-sky-200"
  }
  if (event.transport === "http") {
    return "border-violet-500/30 bg-violet-500/10 text-violet-200"
  }
  if (event.level === "error") {
    return "border-rose-500/30 bg-rose-500/10 text-rose-200"
  }
  if (event.level === "warn") {
    return "border-amber-500/30 bg-amber-500/10 text-amber-200"
  }
  if (event.level === "state") {
    return "border-fuchsia-500/30 bg-fuchsia-500/10 text-fuchsia-200"
  }
  return "border-white/10 bg-white/5 text-slate-200"
}

function renderLogMessage(message: string) {
  const nodes: ReactNode[] = []
  let lastIndex = 0
  let tokenIndex = 0

  for (const match of message.matchAll(LOG_TOKEN_PATTERN)) {
    const [token, packetId, key, quoted, number] = match
    const start = match.index ?? 0

    if (start > lastIndex) {
      nodes.push(
        <span key={`text-${tokenIndex}`}>{message.slice(lastIndex, start)}</span>,
      )
    }

    let className = "text-slate-200"
    if (packetId) {
      className = "font-semibold text-cyan-300"
    } else if (key) {
      className = "text-sky-300"
    } else if (quoted) {
      className = "text-emerald-300"
    } else if (number) {
      className = "text-amber-200"
    }

    nodes.push(
      <span key={`token-${tokenIndex}`} className={className}>
        {token}
      </span>,
    )

    lastIndex = start + token.length
    tokenIndex += 1
  }

  if (lastIndex < message.length) {
    nodes.push(<span key={`tail-${tokenIndex}`}>{message.slice(lastIndex)}</span>)
  }

  return nodes
}

function LogLine({ event }: { event: LogEvent }) {
  return (
    <div className="border-b border-white/5 px-4 py-2 last:border-b-0">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="text-slate-500">{formatLogTimestamp(event.timestamp_ms)}</span>
        <span className={`font-semibold ${logPrefixClass(event)}`}>{logPrefixLabel(event)}</span>
        <span
          className={`rounded-md border px-1.5 py-0.5 text-[10px] uppercase tracking-[0.18em] ${logScopeClass(event)}`}
        >
          {event.scope}
          {event.transport ? ` ${event.transport}` : ""}
        </span>
        {event.session_id ? (
          <span className="rounded-md border border-white/10 bg-white/5 px-1.5 py-0.5 text-[10px] text-slate-300">
            session={event.session_id}
          </span>
        ) : null}
      </div>
      <div className="pt-1 text-slate-200">{renderLogMessage(event.message)}</div>
    </div>
  )
}

function App() {
  const [authKind, setAuthKind] = useState<AuthKind>("android_device")
  const [deviceId, setDeviceId] = useState("")
  const [jwt, setJwt] = useState("")
  const [email, setEmail] = useState("")
  const [password, setPassword] = useState("")
  const [dashboardPassword, setDashboardPassword] = useState("")
  const [dashboardStatus, setDashboardStatus] = useState<DashboardAuthStatus | null>(null)
  const [dashboardToken, setDashboardToken] = useState<string | null>(getAuthToken())
  const [dashboardBusy, setDashboardBusy] = useState(false)
  const [sessions, setSessions] = useState<SessionSnapshot[]>([])
  const [sessionInputs, setSessionInputs] = useState<Record<string, SessionInputs>>({})
  const [minimaps, setMinimaps] = useState<Record<string, MinimapSnapshot | null>>({})
  const [luaStatuses, setLuaStatuses] = useState<Record<string, LuaScriptStatusSnapshot | null>>(
    {},
  )
  const [hoverTiles, setHoverTiles] = useState<Record<string, string>>({})
  const [dropAmounts, setDropAmounts] = useState<Record<string, string>>({})
  const [logs, setLogs] = useState<LogEvent[]>([])
  const [blockNames, setBlockNames] = useState<BlockNameMap>({})
  const [feedback, setFeedback] = useState<Feedback | null>(null)
  const [tutorialCompleted, setTutorialCompleted] =
    useState<TutorialCompletedEvent | null>(null)
  const [connecting, setConnecting] = useState(false)
  const [activeSessionId, setActiveSessionId] = useState<string>("")
  const reconnectTimerRef = useRef<number | null>(null)
  const logViewportRef = useRef<HTMLDivElement | null>(null)
  const shouldAutoScrollLogsRef = useRef(true)

  const dashboardAuthenticated = dashboardStatus?.authenticated ?? false

  const refreshDashboardStatus = useCallback(async () => {
    try {
      const status = await getDashboardAuthStatus()
      setDashboardStatus(status)
      if (!status.authenticated) {
        setDashboardToken(null)
        setAuthToken(null)
      }
    } catch (error) {
      setFeedback({
        kind: "error",
        message: error instanceof Error ? error.message : "dashboard auth failed",
      })
    }
  }, [])

  const upsertSession = useCallback((snapshot: SessionSnapshot) => {
    setSessions((current) => {
      const next = [...current]
      const index = next.findIndex((session) => session.id === snapshot.id)
      if (index === -1) {
        next.push(snapshot)
      } else {
        next[index] = snapshot
      }
      return sortSessions(next)
    })
  }, [])

  const updateFromAction = useCallback((response: ActionResponse) => {
    if (response.session) {
      upsertSession(response.session)
    }
    if (response.result?.message) {
      setFeedback({ kind: "success", message: response.result.message })
    }
  }, [upsertSession])

  useEffect(() => {
    void refreshDashboardStatus()
  }, [refreshDashboardStatus])

  useEffect(() => {
    if (!dashboardAuthenticated) {
      return
    }
    void Promise.all([listSessions(), loadBlockTypes()])
      .then(([sessionPayload, blockPayload]) => {
        setSessions(sortSessions(sessionPayload.sessions ?? []))
        setBlockNames(blockPayload)
      })
      .catch((error: Error) => {
        setFeedback({ kind: "error", message: error.message })
      })
  }, [dashboardAuthenticated])

  useEffect(() => {
    setSessionInputs((current) => {
      const next = { ...current }
      for (const session of sessions) {
        if (!next[session.id]) {
          next[session.id] = {
            ...EMPTY_INPUTS,
            world: session.current_world ?? "",
          }
          continue
        }
        if (!next[session.id].world && session.current_world) {
          next[session.id] = {
            ...next[session.id],
            world: session.current_world,
          }
        }
      }
      return next
    })
  }, [sessions])

  useEffect(() => {
    if (!sessions.length) {
      setActiveSessionId("")
      return
    }

    setActiveSessionId((current) =>
      current && sessions.some((session) => session.id === current)
        ? current
        : sessions[0].id,
    )
  }, [sessions])

  useEffect(() => {
    if (!dashboardAuthenticated) {
      return
    }
    let cancelled = false
    let socket: WebSocket | null = null
    let reconnectDelay = 1000

    const connect = () => {
      if (cancelled) {
        return
      }

      socket = new WebSocket(buildWebSocketUrl(dashboardToken))

      socket.onopen = () => {
        reconnectDelay = 1000
        setFeedback((current) =>
          current?.message === "dashboard websocket disconnected" ? null : current,
        )
      }

      socket.onmessage = (event) => {
        const payload = JSON.parse(event.data) as ServerEvent
        if (payload.type === "log") {
          setLogs((current) => [...current.slice(-499), payload.event])
          return
        }
        if (payload.type === "session") {
          upsertSession(payload.snapshot)
          return
        }
        setTutorialCompleted(payload.event)
      }

      socket.onclose = () => {
        if (cancelled) {
          return
        }
        setFeedback({ kind: "error", message: "dashboard websocket disconnected" })
        reconnectTimerRef.current = window.setTimeout(() => {
          reconnectDelay = Math.min(reconnectDelay * 2, 5000)
          connect()
        }, reconnectDelay)
      }
    }

    connect()

    return () => {
      cancelled = true
      if (reconnectTimerRef.current !== null) {
        window.clearTimeout(reconnectTimerRef.current)
        reconnectTimerRef.current = null
      }
      socket?.close()
    }
  }, [dashboardAuthenticated, dashboardToken, upsertSession])

  const refreshMinimap = useCallback(async (sessionId: string) => {
    try {
      const payload = await getMinimap(sessionId)
      setMinimaps((current) => ({
        ...current,
        [sessionId]: payload.minimap ?? null,
      }))
    } catch {
      setMinimaps((current) => ({
        ...current,
        [sessionId]: null,
      }))
    }
  }, [])

  const refreshLuaStatus = useCallback(async (sessionId: string) => {
    try {
      const payload = await getLuaScriptStatus(sessionId)
      setLuaStatuses((current) => ({
        ...current,
        [sessionId]: payload.status ?? null,
      }))
    } catch {
      setLuaStatuses((current) => ({
        ...current,
        [sessionId]: null,
      }))
    }
  }, [])

  useEffect(() => {
    sessions.forEach((session) => {
      if (
        session.status === "in_world" ||
        session.status === "awaiting_ready" ||
        session.status === "loading_world"
      ) {
        void refreshMinimap(session.id)
      } else if (!session.current_world) {
        setMinimaps((current) => ({ ...current, [session.id]: null }))
      }
    })
  }, [refreshMinimap, sessions])

  useEffect(() => {
    sessions.forEach((session) => {
      void refreshLuaStatus(session.id)
    })
  }, [refreshLuaStatus, sessions])

  const runAction = useCallback(
    async (action: () => Promise<ActionResponse>) => {
      try {
        const response = await action()
        updateFromAction(response)
      } catch (error) {
        if (error instanceof Error && error.message.toLowerCase().includes("unauthorized")) {
          await refreshDashboardStatus()
        }
        setFeedback({
          kind: "error",
          message: error instanceof Error ? error.message : "request failed",
        })
      }
    },
    [refreshDashboardStatus, updateFromAction],
  )

  const runLuaAction = useCallback(
    async (
      sessionId: string,
      action: () => Promise<{ result?: { message?: string }; status?: LuaScriptStatusSnapshot | null }>,
    ) => {
      try {
        const response = await action()
        if (response.result?.message) {
          setFeedback({ kind: "success", message: response.result.message })
        }
        setLuaStatuses((current) => ({
          ...current,
          [sessionId]: response.status ?? current[sessionId] ?? null,
        }))
        await refreshLuaStatus(sessionId)
      } catch (error) {
        if (error instanceof Error && error.message.toLowerCase().includes("unauthorized")) {
          await refreshDashboardStatus()
        }
        setFeedback({
          kind: "error",
          message: error instanceof Error ? error.message : "request failed",
        })
      }
    },
    [refreshDashboardStatus, refreshLuaStatus],
  )

  useEffect(() => {
    const viewport = logViewportRef.current
    if (!viewport || !shouldAutoScrollLogsRef.current) {
      return
    }
    viewport.scrollTop = viewport.scrollHeight
  }, [logs])

  const createAuthInput = useMemo<AuthInput>(() => {
    if (authKind === "jwt") {
      return {
        kind: "jwt",
        jwt,
        device_id: deviceId || null,
      }
    }
    if (authKind === "email_password") {
      return {
        kind: "email_password",
        email,
        password,
        device_id: deviceId || null,
      }
    }
    return {
      kind: "android_device",
      device_id: deviceId || null,
    }
  }, [authKind, deviceId, email, jwt, password])

  const handleConnect = async () => {
    setConnecting(true)
    try {
      const response = await connectWithAuth(createAuthInput)
      updateFromAction(response)
    } catch (error) {
      setFeedback({
        kind: "error",
        message: error instanceof Error ? error.message : "connect failed",
      })
    } finally {
      setConnecting(false)
    }
  }

  const handleDashboardRegister = async () => {
    setDashboardBusy(true)
    try {
      const response = await registerDashboardPassword(dashboardPassword)
      if (response.token) {
        setAuthToken(response.token)
        setDashboardToken(response.token)
      }
      await refreshDashboardStatus()
      setDashboardPassword("")
      if (response.message) {
        setFeedback({ kind: "success", message: response.message })
      }
    } catch (error) {
      setFeedback({
        kind: "error",
        message: error instanceof Error ? error.message : "register failed",
      })
    } finally {
      setDashboardBusy(false)
    }
  }

  const handleDashboardLogin = async () => {
    setDashboardBusy(true)
    try {
      const response = await loginDashboard(dashboardPassword)
      if (response.token) {
        setAuthToken(response.token)
        setDashboardToken(response.token)
      }
      await refreshDashboardStatus()
      setDashboardPassword("")
      if (response.message) {
        setFeedback({ kind: "success", message: response.message })
      }
    } catch (error) {
      setFeedback({
        kind: "error",
        message: error instanceof Error ? error.message : "login failed",
      })
    } finally {
      setDashboardBusy(false)
    }
  }

  const handleDashboardLogout = async () => {
    setDashboardBusy(true)
    try {
      await logoutDashboard()
    } catch {
      // ignore
    } finally {
      setAuthToken(null)
      setDashboardToken(null)
      await refreshDashboardStatus()
      setDashboardBusy(false)
    }
  }

  if (!dashboardStatus || !dashboardAuthenticated) {
    const registered = dashboardStatus?.registered ?? false
    return (
      <div className="min-h-screen bg-[radial-gradient(circle_at_top,#14212f_0%,#0d141c_45%,#081018_100%)] text-foreground">
        <div className="mx-auto flex min-h-screen max-w-[640px] items-center px-4 py-10">
          <Card className="w-full border-white/10 bg-card/90 ring-white/10">
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Moon className="size-4" />
                Moonlight Dashboard Locked
              </CardTitle>
              <CardDescription>
                {registered
                  ? "Enter your password to unlock the dashboard."
                  : "Create a password to protect the dashboard."}
              </CardDescription>
            </CardHeader>
            <CardContent className="grid gap-4">
              <div className="grid gap-2">
                <label className="text-xs text-muted-foreground">Password</label>
                <Input
                  type="password"
                  value={dashboardPassword}
                  onChange={(event) => setDashboardPassword(event.target.value)}
                  placeholder={registered ? "Your password" : "Create a strong password"}
                  className="rounded-xl border-white/10 bg-white/5"
                />
              </div>
              <Button
                onClick={() => void (registered ? handleDashboardLogin() : handleDashboardRegister())}
                disabled={dashboardBusy}
                className="rounded-xl"
              >
                {dashboardBusy ? (
                  <SpinnerGap className="size-4 animate-spin" />
                ) : (
                  <Plug className="size-4" />
                )}
                {registered ? "Unlock Dashboard" : "Create Password"}
              </Button>
              {feedback ? (
                <div
                  className={`rounded-xl border px-3 py-2 text-xs ${
                    feedback.kind === "error"
                      ? "border-red-500/30 bg-red-500/10 text-red-100"
                      : "border-emerald-500/30 bg-emerald-500/10 text-emerald-100"
                  }`}
                >
                  {feedback.message}
                </div>
              ) : null}
            </CardContent>
          </Card>
        </div>
      </div>
    )
  }

  const mutateSessionInput = (
    sessionId: string,
    field: keyof SessionInputs,
    value: string,
  ) => {
    setSessionInputs((current) => ({
      ...current,
      [sessionId]: {
        ...(current[sessionId] ?? EMPTY_INPUTS),
        [field]: value,
      },
    }))
  }

  const parseSpamDelayMs = (rawValue: string) => {
    const seconds = Number(rawValue)
    if (!Number.isFinite(seconds) || seconds <= 0) {
      throw new Error("spam delay must be a number greater than 0 seconds")
    }
    const delayMs = Math.round(seconds * 1000)
    if (delayMs < 250) {
      throw new Error("spam delay must be at least 0.25 seconds")
    }
    return delayMs
  }

  const formatLuaTime = (timestampMs: number | null) =>
    timestampMs ? new Date(timestampMs).toISOString().replace("T", " ").replace("Z", " UTC") : "-"

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top,#14212f_0%,#0d141c_45%,#081018_100%)] text-foreground">
      <div className="mx-auto grid max-w-[1800px] gap-4 px-3 py-4 sm:gap-6 sm:px-4 sm:py-6 xl:grid-cols-[340px_minmax(0,1fr)] 2xl:grid-cols-[360px_minmax(0,1fr)]">
        <Card className="border-white/10 bg-card/90 ring-white/10 xl:sticky xl:top-4 xl:self-start">
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Moon className="size-4" />
              Moonlight Dashboard
            </CardTitle>
            <CardDescription>
              Create sessions, then manage every bot from its own panel.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4">
            <div className="grid gap-2">
              <label className="text-xs text-muted-foreground">Auth Kind</label>
              <Select
                value={authKind}
                onValueChange={(value) => setAuthKind(value as AuthKind)}
              >
                <SelectTrigger className="w-full rounded-xl border-white/10 bg-white/5">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="android_device">Android Device</SelectItem>
                  <SelectItem value="jwt">JWT</SelectItem>
                  <SelectItem value="email_password">Email + Password</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <div className="grid gap-2">
              <label className="text-xs text-muted-foreground">Device ID</label>
              <div className="grid gap-2 sm:grid-cols-[1fr_auto]">
                <Input
                  value={deviceId}
                  onChange={(event) => setDeviceId(event.target.value)}
                  placeholder="57ce..."
                  className="rounded-xl border-white/10 bg-white/5"
                />
                <Button
                  variant="outline"
                  className="rounded-xl border-white/10 bg-white/5"
                  onClick={() => setDeviceId(generateDeviceId())}
                >
                  Generate
                </Button>
              </div>
            </div>

            {authKind === "jwt" ? (
              <div className="grid gap-2">
                <label className="text-xs text-muted-foreground">JWT</label>
                <Textarea
                  value={jwt}
                  onChange={(event) => setJwt(event.target.value)}
                  rows={6}
                  placeholder="eyJ..."
                  className="rounded-xl border-white/10 bg-white/5"
                />
              </div>
            ) : null}

            {authKind === "email_password" ? (
              <div className="grid gap-4">
                <div className="grid gap-2">
                  <label className="text-xs text-muted-foreground">Email</label>
                  <Input
                    value={email}
                    onChange={(event) => setEmail(event.target.value)}
                    placeholder="you@example.com"
                    className="rounded-xl border-white/10 bg-white/5"
                  />
                </div>
                <div className="grid gap-2">
                  <label className="text-xs text-muted-foreground">Password</label>
                  <Input
                    type="password"
                    value={password}
                    onChange={(event) => setPassword(event.target.value)}
                    className="rounded-xl border-white/10 bg-white/5"
                  />
                </div>
              </div>
            ) : null}

            <Button
              onClick={() => void handleConnect()}
              disabled={connecting}
              className="rounded-xl"
            >
              {connecting ? <SpinnerGap className="size-4 animate-spin" /> : <Plug className="size-4" />}
              Connect
            </Button>
            <Button
              variant="outline"
              onClick={() => void handleDashboardLogout()}
              disabled={dashboardBusy}
              className="rounded-xl border-white/10 bg-white/5"
            >
              {dashboardBusy ? <SpinnerGap className="size-4 animate-spin" /> : <Plug className="size-4" />}
              Lock Dashboard
            </Button>

            {feedback ? (
              <div
                className={`rounded-xl border px-3 py-2 text-xs ${
                  feedback.kind === "error"
                    ? "border-red-500/30 bg-red-500/10 text-red-100"
                    : "border-emerald-500/30 bg-emerald-500/10 text-emerald-100"
                }`}
              >
                {feedback.message}
              </div>
            ) : null}
          </CardContent>
        </Card>

        <div className="grid gap-6">
          <Card className="border-white/10 bg-card/90 ring-white/10">
            <CardHeader>
              <CardTitle className="flex items-center gap-2">
                <Bug className="size-4" />
                Live Log
              </CardTitle>
            </CardHeader>
            <CardContent>
              <ScrollArea
                className="h-44 rounded-xl border border-white/10 bg-[#081018] sm:h-56"
                viewportRef={logViewportRef}
                viewportProps={{
                  onScroll: (event) => {
                    const target = event.currentTarget
                    const distanceFromBottom =
                      target.scrollHeight - target.scrollTop - target.clientHeight
                    shouldAutoScrollLogsRef.current = distanceFromBottom < 12
                  },
                }}
              >
                <div className="font-mono text-[11px] leading-5">
                  {logs.length ? (
                    logs.map((event, index) => (
                      <LogLine
                        key={`${event.timestamp_ms}-${event.scope}-${event.session_id ?? "global"}-${index}`}
                        event={event}
                      />
                    ))
                  ) : (
                    <div className="px-4 py-3 text-slate-400">No events yet.</div>
                  )}
                </div>
              </ScrollArea>
            </CardContent>
          </Card>

          <div className="grid gap-4">
            {sessions.length ? (
              <Tabs
                value={activeSessionId}
                onValueChange={setActiveSessionId}
                className="gap-4"
              >
                <div className="-mx-1 overflow-x-auto px-1">
                  <TabsList
                    variant="line"
                    className="flex w-max min-w-full gap-2 rounded-2xl border border-white/10 bg-card/90 p-2"
                  >
                    {sessions.map((session) => (
                      <TabsTrigger
                        key={session.id}
                        value={session.id}
                        className="flex-none rounded-xl border border-white/10 bg-white/5 px-3 py-2 data-active:bg-white/10"
                      >
                        <div className="flex items-center gap-2">
                          <span>{session.id}</span>
                          <Badge
                            variant={statusVariant(session.status)}
                            className="rounded-full px-2"
                          >
                            {session.status}
                          </Badge>
                        </div>
                      </TabsTrigger>
                    ))}
                  </TabsList>
                </div>

                {sessions.map((session) => {
                  const inputs = sessionInputs[session.id] ?? EMPTY_INPUTS
                  const minimap = minimaps[session.id] ?? null
                  return (
                    <TabsContent key={session.id} value={session.id} className="mt-0">
                      <Card className="border-white/10 bg-card/90 ring-white/10">
                        <CardHeader>
                          <div className="flex flex-col gap-3 sm:flex-row sm:flex-wrap sm:items-start sm:justify-between">
                            <div className="grid gap-1">
                              <CardTitle className="flex items-center gap-2">
                                <Plug className="size-4" />
                                {session.id}
                              </CardTitle>
                              <CardDescription>
                                user={session.username ?? "-"} world={session.current_world ?? "-"} host=
                                {session.current_host}:{session.current_port}
                              </CardDescription>
                              <div className="text-xs text-muted-foreground">
                                position map=({session.player_position.map_x ?? "-"},{" "}
                                {session.player_position.map_y ?? "-"}) world=(
                                {session.player_position.world_x ?? "-"},{" "}
                                {session.player_position.world_y ?? "-"}) ping=
                                {session.ping_ms != null ? `${session.ping_ms}ms` : "-"}
                              </div>
                              {session.last_error ? (
                                <div className="text-xs text-red-200">{session.last_error}</div>
                              ) : null}
                            </div>
                            <Badge
                              variant={statusVariant(session.status)}
                              className="rounded-full px-3"
                            >
                              {session.status}
                            </Badge>
                          </div>
                        </CardHeader>

                        <CardContent className="grid gap-4">
                          <div className="grid gap-3 xl:grid-cols-[minmax(320px,1.15fr)_minmax(320px,.85fr)]">
                            <div className="grid gap-3">
                              <div className="grid gap-2 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <Waves className="size-3.5" />
                              Minimap
                            </div>
                            <MinimapPanel
                              minimap={minimap}
                              playerPosition={session.player_position}
                              onHoverChange={(value) =>
                                setHoverTiles((current) => ({
                                  ...current,
                                  [session.id]: value,
                                }))
                              }
                            />
                            <div className="text-xs text-muted-foreground">
                              {hoverTiles[session.id] ?? "Hover a tile to inspect it."}
                            </div>
                              </div>

                              <div className="grid gap-2 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <Gear className="size-3.5" />
                              Inventory
                            </div>
                            <div className="flex flex-wrap gap-2">
                              {session.inventory.length ? (
                                session.inventory.map((item) => {
                                  const blockName = blockNames[String(item.block_id)]
                                  const label =
                                    blockName && blockName !== "None"
                                      ? camelToWords(blockName)
                                      : `#${item.block_id}`
                                  return (
                                    <div
                                      key={`${session.id}-${item.block_id}-${item.inventory_type}`}
                                      className="grid min-w-[92px] flex-1 gap-1 rounded-xl border border-white/10 bg-white/4 p-2.5 text-center sm:min-w-[110px] sm:flex-none sm:p-3"
                                    >
                                      <TileSprite
                                        blockId={item.block_id}
                                        size={40}
                                        className="mx-auto"
                                        fallback={<span className="text-[9px] text-muted-foreground">?</span>}
                                      />
                                      <div className="font-mono text-[11px] text-muted-foreground">
                                        #{item.block_id}
                                      </div>
                                      <div className="text-xs font-medium">{label}</div>
                                      <div className="text-lg font-semibold text-cyan-300">
                                        x{item.amount}
                                      </div>
                                      <div className="text-[11px] text-muted-foreground">
                                        {getItemCategory(blockName, inventoryTypeLabel(item.inventory_type))}
                                      </div>
                                      {CLOTHING_TYPES.has(item.inventory_type) ? (
                                        <div className="grid grid-cols-2 gap-2">
                                          <Button
                                            size="xs"
                                            variant="outline"
                                            className="rounded-lg border-white/10 bg-white/5"
                                            onClick={() =>
                                              void runAction(() =>
                                                wearItem(session.id, item.block_id, true),
                                              )
                                            }
                                          >
                                            Wear
                                          </Button>
                                          <Button
                                            size="xs"
                                            variant="outline"
                                            className="rounded-lg border-white/10 bg-white/5"
                                            onClick={() =>
                                              void runAction(() =>
                                                wearItem(session.id, item.block_id, false),
                                              )
                                            }
                                          >
                                            Off
                                          </Button>
                                        </div>
                                      ) : null}
                                      <div className="flex gap-1">
                                        <Input
                                          className="h-6 min-w-0 flex-1 rounded-lg border-white/10 bg-white/5 px-1.5 text-center text-xs"
                                          type="number"
                                          min={1}
                                          max={item.amount}
                                          value={dropAmounts[`${session.id}-${item.block_id}-${item.inventory_type}`] ?? "1"}
                                          onChange={(e) => {
                                            const key = `${session.id}-${item.block_id}-${item.inventory_type}`
                                            setDropAmounts((prev) => ({ ...prev, [key]: e.target.value }))
                                          }}
                                        />
                                        <Button
                                          size="xs"
                                          variant="outline"
                                          className="rounded-lg border-white/10 bg-white/5"
                                          onClick={() => {
                                            const key = `${session.id}-${item.block_id}-${item.inventory_type}`
                                            const amt = Math.max(1, Math.min(item.amount, parseInt(dropAmounts[key] ?? "1", 10) || 1))
                                            void runAction(() =>
                                              dropItem(session.id, item.block_id, item.inventory_type, amt),
                                            )
                                          }}
                                        >
                                          Drop
                                        </Button>
                                      </div>
                                    </div>
                                  )
                                })
                              ) : (
                                <div className="rounded-xl border border-dashed border-white/10 px-4 py-5 text-xs text-muted-foreground">
                                  Inventory empty.
                                </div>
                              )}
                            </div>
                              </div>
                            </div>

                            <div className="grid gap-3">
                              <div className="grid gap-3 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <NavigationArrow className="size-3.5" />
                              World Controls
                            </div>
                            <div className="grid gap-2 sm:grid-cols-[1fr_auto]">
                              <Input
                                value={inputs.world}
                                onChange={(event) =>
                                  mutateSessionInput(session.id, "world", event.target.value)
                                }
                                placeholder="World name"
                                className="rounded-xl border-white/10 bg-white/5"
                              />
                              <Button
                                className="rounded-xl"
                                onClick={() =>
                                  void runAction(() => joinWorld(session.id, inputs.world))
                                }
                              >
                                Join
                              </Button>
                            </div>
                            <div className="grid grid-cols-1 gap-2 sm:grid-cols-4">
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() => void runAction(() => leaveWorld(session.id))}
                              >
                                Leave
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runAction(() => disconnectSession(session.id))
                                }
                              >
                                Disconnect
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runAction(() => reconnectSession(session.id))
                                }
                              >
                                Reconnect
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runAction(() => automateTutorial(session.id))
                                }
                              >
                                Tutorial
                              </Button>
                            </div>
                              </div>

                              <div className="grid gap-3 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <GameController className="size-3.5" />
                              Joystick
                            </div>
                            <div className="grid gap-2">
                              <div className="grid grid-cols-3 gap-2">
                                <div />
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() =>
                                    void runAction(() => moveSession(session.id, "up"))
                                  }
                                >
                                  Up
                                </Button>
                                <div />
                              </div>
                              <div className="grid grid-cols-3 gap-2">
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() =>
                                    void runAction(() => moveSession(session.id, "left"))
                                  }
                                >
                                  Left
                                </Button>
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() =>
                                    void runAction(() => moveSession(session.id, "down"))
                                  }
                                >
                                  Down
                                </Button>
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() =>
                                    void runAction(() => moveSession(session.id, "right"))
                                  }
                                >
                                  Right
                                </Button>
                              </div>
                            </div>
                              </div>

                              <div className="grid gap-3 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <Fish className="size-3.5" />
                              Auto Fishing
                            </div>
                            <Input
                              value={inputs.bait}
                              onChange={(event) =>
                                mutateSessionInput(session.id, "bait", event.target.value)
                              }
                              placeholder="Bait name or lure name"
                              className="rounded-xl border-white/10 bg-white/5"
                            />
                            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runAction(() =>
                                    startFishing(session.id, "left", inputs.bait),
                                  )
                                }
                              >
                                Fish Left
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runAction(() =>
                                    startFishing(session.id, "right", inputs.bait),
                                  )
                                }
                              >
                                Fish Right
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() => void runAction(() => stopFishing(session.id))}
                              >
                                Stop
                              </Button>
                            </div>
                              </div>

                              <div className="grid gap-3 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <ChatCenteredDots className="size-3.5" />
                              Chat And Spam
                            </div>
                            <div className="grid gap-2">
                              <Input
                                value={inputs.chat}
                                onChange={(event) =>
                                  mutateSessionInput(session.id, "chat", event.target.value)
                                }
                                placeholder="Say something in world chat"
                                className="rounded-xl border-white/10 bg-white/5"
                              />
                              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() => void runAction(() => talk(session.id, inputs.chat))}
                                >
                                  Send Chat
                                </Button>
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() => mutateSessionInput(session.id, "chat", "")}
                                >
                                  Clear
                                </Button>
                              </div>
                            </div>
                            <div className="grid gap-2">
                              <div className="text-xs text-muted-foreground">
                                Repeats the same world chat message with your chosen delay.
                              </div>
                              <div className="grid gap-2 sm:grid-cols-[1fr_120px]">
                                <Input
                                  value={inputs.spam}
                                  onChange={(event) =>
                                    mutateSessionInput(session.id, "spam", event.target.value)
                                  }
                                  placeholder="Spam message"
                                  className="rounded-xl border-white/10 bg-white/5"
                                />
                                <Input
                                  type="number"
                                  min="0.25"
                                  step="0.25"
                                  value={inputs.spamDelay}
                                  onChange={(event) =>
                                    mutateSessionInput(session.id, "spamDelay", event.target.value)
                                  }
                                  placeholder="5"
                                  className="rounded-xl border-white/10 bg-white/5"
                                />
                              </div>
                              <div className="text-[11px] text-muted-foreground">
                                Delay in seconds.
                              </div>
                              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() =>
                                    void runAction(() =>
                                      startSpam(
                                        session.id,
                                        inputs.spam,
                                        parseSpamDelayMs(inputs.spamDelay),
                                      ),
                                    )
                                  }
                                >
                                  Start Spam
                                </Button>
                                <Button
                                  variant="outline"
                                  className="rounded-xl border-white/10 bg-white/5"
                                  onClick={() => void runAction(() => stopSpam(session.id))}
                                >
                                  Stop Spam
                                </Button>
                              </div>
                              </div>
                            </div>
                              <div className="grid gap-3 rounded-2xl border border-white/10 bg-white/3 p-3">
                            <div className="flex items-center gap-2 text-xs uppercase tracking-[0.2em] text-muted-foreground">
                              <Bug className="size-3.5" />
                              Lua Script
                            </div>
                            <Textarea
                              value={inputs.luaSource}
                              onChange={(event) =>
                                mutateSessionInput(session.id, "luaSource", event.target.value)
                              }
                              rows={10}
                              placeholder={"bot:talk(\"hello\")\nbot:sleep(500)\nlocal world = bot:getWorld()"}
                              className="rounded-xl border-white/10 bg-white/5 font-mono text-[11px]"
                            />
                            <div className="grid gap-1 rounded-xl border border-white/10 bg-white/4 p-3 text-xs text-muted-foreground">
                              <div>
                                Status:{" "}
                                <span
                                  className={
                                    luaStatuses[session.id]?.running
                                      ? "text-emerald-300"
                                      : "text-slate-200"
                                  }
                                >
                                  {luaStatuses[session.id]?.running ? "running" : "idle"}
                                </span>
                              </div>
                              <div>
                                Started: {formatLuaTime(luaStatuses[session.id]?.started_at ?? null)}
                              </div>
                              <div>
                                Finished: {formatLuaTime(luaStatuses[session.id]?.finished_at ?? null)}
                              </div>
                              <div>
                                Last Result: {luaStatuses[session.id]?.last_result_message ?? "-"}
                              </div>
                              <div
                                className={
                                  luaStatuses[session.id]?.last_error ? "text-rose-300" : undefined
                                }
                              >
                                Last Error: {luaStatuses[session.id]?.last_error ?? "-"}
                              </div>
                            </div>
                            <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runLuaAction(session.id, () =>
                                    startLuaScript(session.id, inputs.luaSource),
                                  )
                                }
                              >
                                Run Script
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() =>
                                  void runLuaAction(session.id, () => stopLuaScript(session.id))
                                }
                              >
                                Stop Script
                              </Button>
                              <Button
                                variant="outline"
                                className="rounded-xl border-white/10 bg-white/5"
                                onClick={() => void refreshLuaStatus(session.id)}
                              >
                                Refresh
                              </Button>
                            </div>
                              </div>
                            </div>
                          </div>
                        </CardContent>
                      </Card>
                    </TabsContent>
                  )
                })}
              </Tabs>
            ) : (
              <Card className="border-white/10 bg-card/90 ring-white/10">
                <CardContent className="px-4 py-8 text-center text-sm text-muted-foreground">
                  No sessions yet. Create a bot from the panel on the left.
                </CardContent>
              </Card>
            )}
          </div>
        </div>
      </div>

      <Dialog
        open={tutorialCompleted !== null}
        onOpenChange={(open) => {
          if (!open) {
            setTutorialCompleted(null)
          }
        }}
      >
        <DialogContent className="rounded-2xl border-white/10 bg-card text-sm">
          <DialogHeader>
            <DialogTitle>Tutorial Finished</DialogTitle>
            <DialogDescription>
              {tutorialCompleted?.message ?? "Tutorial finished."}
            </DialogDescription>
          </DialogHeader>
          <DialogFooter showCloseButton />
        </DialogContent>
      </Dialog>
    </div>
  )
}

export default App
