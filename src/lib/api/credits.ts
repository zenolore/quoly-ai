import { invoke } from "@tauri-apps/api/core";

export type CreditPlatform = "workbuddy" | "traework" | "qoder" | "minimax-code";
export type CreditRegion = "cn" | "intl";

export interface Credentials {
  region?: CreditRegion;
  accessToken: string;
  refreshToken?: string | null;
  userId?: string | null;
  deviceId?: string | null;
  expiresAt?: number | null;
  extra: unknown;
}

export interface CreditPack {
  unit?: string;
  id: string;
  name: string;
  remaining: number | null;
  expiresAt: number | null;
}

/**
 * Live health of one model route, as reported by the running proxy.
 * Mirrors `ModelHealthReport` in the Rust backend.
 */
export interface ModelHealthReport {
  /** Fully-qualified route, e.g. `workbuddy/kimi-k3.1-pro`. */
  route: string;
  /** `workbuddy` | `qoder` | `traework` | ... */
  platform: string;
  model: string;
  totalOk: number;
  totalFail: number;
  /** Client-aborted / timed-out requests, excluded from the success rate. */
  totalCancelled: number;
  /** Success rate over recent attempts (0–1); null until the route is tried. */
  recentRate: number | null;
  /** Success rate over every attempt ever served — matches the ok/fail counters. */
  successRate: number | null;
  avgLatencyMs: number | null;
  /** Remaining quarantine in ms; null means usable now. */
  quarantinedMs: number | null;
  lastError: string | null;
}

export interface CreditModel {
  id: string;
  upstreamId?: string | null;
  upstreamModelIds?: string[];
  name: string;
  canonicalId: string | null;
  supportsTools: boolean;
  contextWindow: number | null;
  contextWindowOptions?: number[];
  maxOutputTokens?: number | null;
  supportsImages?: boolean | null;
  supportsReasoning?: boolean | null;
  reasoningEffortOptions?: string[];
  defaultReasoningEffort?: string | null;
  verified: boolean;
  verificationNote: string | null;
}

export interface CreditReward {
  id: string;
  name: string;
  kind: string;
  status: string;
  amount: number | null;
  nextAvailableAt: number | null;
  actionUrl: string | null;
}

export interface CreditSnapshot {
  balance: number | null;
  unit: string;
  quota: string | null;
  packs: CreditPack[];
  rewards: CreditReward[];
  models: CreditModel[];
  fetchedAt: number;
  stale: boolean;
  diagnostic: string | null;
  sync?: {
    rewardsFetchedAt: number | null;
    modelsFetchedAt: number | null;
    rewardsError: string | null;
    modelsError: string | null;
  };
}

export interface CreditAccount {
  id: string;
  platform: CreditPlatform;
  region: CreditRegion;
  userId: string;
  label: string;
  enabled: boolean;
  status: string;
  lastError: string | null;
  snapshot: CreditSnapshot | null;
}

export interface RewardOutcome {
  accountId: string;
  rewardId: string;
  name: string;
  status: string;
  amount: number | null;
  kind: string;
  message: string;
  createdAt: number;
}

/** One day of growth totals in the history table. */
export interface GrowthDay {
  date: string;
  credits: number;
  accounts: number;
  runs: number;
  tasksClaimed: number;
  travelClaimed: number;
  buddyBoxes: number;
}

/** One day in the overview trend chart. */
export interface UsageDay {
  /** Beijing-local `YYYY-MM-DD`. */
  date: string;
  /** Prompt tokens served from cache. */
  cachedTokens: number;
  /** Cache misses — the part that actually bills. */
  newPromptTokens: number;
  /**
   * Prompt tokens from records written before the cache field existed. Kept
   * separate so the cache-write stack never implies a measured figure.
   */
  unmeasuredPromptTokens: number;
  completionTokens: number;
  /** Request count — drawn as the dashed line. */
  requests: number;
  succeeded: number;
  failed: number;
  avgLatencyMs: number | null;
}

/** Whether an account can take part in growth automation yet. */
export interface GrowthAccountState {
  id: string;
  label: string;
  hasBuddy: boolean;
  energy: number;
  travelState: string;
  claimedTasks: number;
  pendingTasks: number;
  ready: boolean;
  hint: string;
}

/** Result of attempting to unlock a buddy-less account. */
export interface ActivationResult {
  accountId: string;
  label: string;
  /** skipped_has_buddy | skipped_today | reported | activated | failed */
  outcome: string;
  streakBefore: number;
  streakAfter: number;
  agreed: boolean;
  adopted: boolean;
  credit: number;
  energy: number;
  note: string;
}

/** 账号文件解析结果。 */
export interface ParsedAccount {
  nickname: string;
  userId: string;
  accessToken: string;
  problems: string[];
  existingId: string | null;
}

export interface ParseReport {
  format: string;
  accounts: ParsedAccount[];
  duplicates: number;
  skipped: string[];
}

export interface ImportOutcome {
  nickname: string;
  label: string;
  ok: boolean;
  message: string;
}

export interface ProxySettings {
  port: number;
  autoClaim: boolean;
  autoRefresh: boolean;
  /** Run the WorkBuddy growth-center pass alongside the daily claim check. */
  autoGrowth: boolean;
}

/** Result of one growth-center pass for one account. */
export interface GrowthOutcome {
  checkinCredits: number;
  checkinStatus: string;
  travelClaimed: number;
  travelDispatched: boolean;
  travelState: string;
  tasksAccepted: number;
  tasksClaimed: number;
  taskCredits: number;
  streakTiers: string[];
  makeupUsed: boolean;
  lotteryDraws: number;
  buddyBoxes: number;
  steps: string[];
}

/** Read-only growth snapshot for one account. */
export interface GrowthSnapshot {
  level: number;
  completed: number;
  total: number;
  energy: number;
  tasks: {
    code: string;
    title: string;
    description: string;
    status: string;
    rewardCredit: number;
    rewardEnergy: number;
    kind: string;
    locked: boolean;
    jumpUrl: string;
  }[];
  travelState: string;
  travelRemainingSeconds: number;
  dailyLimitReached: boolean;
  streakDays: number;
  nextTier: string;
  nextTierRemaining: number;
  lotteryChances: number;
  buddyAffordable: number;
}

export interface GatewayStatus {
  running: boolean;
  port: number;
  baseUrl: string;
  lastError: string | null;
}

export interface ProxyRequest {
  id: string;
  accountId: string;
  platform: CreditPlatform;
  region: CreditRegion;
  model: string;
  elapsedMs: number;
  inputTokens: number | null;
  outputTokens: number | null;
  status: string;
  createdAt: number;
  diagnostic?: {
    httpStatus: number;
    upstreamCode: string | null;
    upstreamRequestId: string | null;
    stage: string;
    message: string;
  } | null;
}

export interface PlatformUsage {
  platform: string;
  requests: number;
  succeeded: number;
  failed: number;
  promptTokens: number;
  billablePromptTokens: number;
  completionTokens: number;
  totalTokens: number;
  billableTokens: number;
  /** Hits / prompt tokens that reported a cache figure. */
  cacheHitRate: number | null;
  /** Prompt tokens on records that reported a cache figure. */
  promptTokensWithCache: number;
  billablePromptTokensWithCache: number;
  avgLatencyMs: number | null;
  p50LatencyMs: number | null;
}

export interface ClaimAccountRow {
  accountId: string;
  name: string;
  /** claimed | already_claimed | pending | unavailable | failed | none */
  status: string;
  amount: number;
  reward: string | null;
  message: string | null;
}

export interface PlatformClaims {
  platform: string;
  accounts: number;
  recorded: number;
  claimed: number;
  alreadyClaimed: number;
  unavailable: number;
  pending: number;
  missing: number;
  credits: number;
  successRate: number | null;
  rows: ClaimAccountRow[];
}

export interface UsageStats {
  todayRequests: number;
  totalRequests: number;
  todayPromptTokens: number;
  todayBillablePromptTokens: number;
  todayCompletionTokens: number;
  /** Prompt tokens today on records that reported a cache figure. */
  todayPromptTokensWithCache: number;
  todayBillablePromptTokensWithCache: number;
  /** Honest hit rate: hits / prompt tokens **that reported a cache figure**. */
  todayCacheHitRate: number | null;
  /** Share of today's prompt tokens the hit rate actually covers (0–1). */
  todayCacheCoverage: number | null;
  /** Credits actually spent today, summed from the per-request upstream figure. */
  todaySpentCredits: number;
  /** How many of today's requests reported a credit value (TRAE/Qoder do not). */
  todayCreditCoverage: number;
  monthSpentCredits: number;
  /** 本程序今天领到的奖励合计（只含经它自己之手领取的）。 */
  todayClaimedCredits: number;
  /**
   * 今天新发放资源包的总额，不区分领取者。
   * 和 todayClaimedCredits 的差额就是「别的客户端领的 / 任务发的整包」。
   */
  todayNewPackCredits: number;
  todayClaimingAccounts: number;
  todayPendingClaims: number;
  todaySucceeded: number;
  todayFailed: number;
  todayAvgLatencyMs: number | null;
  monthPromptTokens: number;
  monthBillablePromptTokens: number;
  monthCompletionTokens: number;
  monthRequests: number;
  monthClaimedCredits: number;
  truncated: boolean;
  /** 这批统计是什么时候算出来的（epoch ms），用来在界面上标注数据时间。 */
  sampledAt: number;
  claims?: PlatformClaims[];
  /** Per-platform totals for TODAY (request counts, tokens, latency). */
  platforms?: PlatformUsage[];
}

export interface CreditDashboard {
  accounts: CreditAccount[];
  settings: ProxySettings;
  gateway: GatewayStatus;
  rewards: RewardOutcome[];
  requests: ProxyRequest[];
  refreshingAccountIds: string[];
  /**
   * Full-window totals aggregated server-side. `requests` / `rewards` above are
   * capped at 100 rows for display, so any card showing a number must read from
   * here instead.
   */
  usage?: UsageStats;
}

export interface OAuthStart {
  flowId: string;
  authorizationUrl: string;
  expiresAt: number;
}

export type OAuthResult =
  | { status: "pending"; message?: string }
  | { status: "completed"; account: CreditAccount; message?: string }
  | { status: "failed"; message: string };

export const creditsApi = {
  dashboard: () => invoke<CreditDashboard>("credits_dashboard"),
  setViewActive: (active: boolean) => invoke<void>("credits_set_view_active", { active }),
  networkRestored: () => invoke<void>("credits_network_restored"),
  importLocal: (platform: CreditPlatform, region: CreditRegion = "cn") =>
    invoke<CreditAccount>("credits_import_local", { platform, region }),
  /** 解析账号文件，只回报内容，不写库。 */
  parseAccountFile: (content: string) =>
    invoke<ParseReport>("credits_parse_account_file", { content }),
  /** 真正导入。逐个校验凭据，失败的不会落库。 */
  importAccounts: (content: string, labelSuffix?: string) =>
    invoke<ImportOutcome[]>("credits_import_accounts", { content, labelSuffix }),
  setAccountEnabled: (accountId: string, enabled: boolean) =>
    invoke<void>("credits_set_account_enabled", { accountId, enabled }),
  deleteAccount: (accountId: string) => invoke<void>("credits_delete_account", { accountId }),
  refresh: (accountId?: string) => invoke<CreditAccount[]>("credits_refresh", { accountId }),
  claim: (accountId?: string) => invoke<RewardOutcome[]>("credits_claim", { accountId }),
  /** Run the WorkBuddy growth-center automation (travel/tasks/streak/boxes). */
  runGrowth: (accountId?: string) =>
    invoke<GrowthOutcome[]>("credits_run_growth", { accountId }),
  /** Read-only growth snapshot for one account. */
  growthSnapshot: (accountId: string) =>
    invoke<GrowthSnapshot>("credits_growth_snapshot", { accountId }),
  /** Per-day growth totals, newest first. */
  growthDaily: (days?: number) =>
    invoke<GrowthDay[]>("credits_growth_daily", { days }),
  /** Per-day token / request totals for the trend chart, oldest first. */
  usageDaily: (days?: number) =>
    invoke<UsageDay[]>("credits_usage_daily", { days }),
  /** Which accounts are ready to earn, and why the others are not. */
  growthAccounts: () =>
    invoke<GrowthAccountState[]>("credits_growth_accounts"),
  /**
   * Unlock buddy-less accounts (one telemetry event each, 3-6 min apart).
   * Requires an explicit list; capped at 5 per call by the backend.
   */
  activateAccounts: (accountIds: string[]) =>
    invoke<ActivationResult[]>("credits_activate_accounts", { accountIds }),
  /** Recent activation attempts, newest first, for the audit view. */
  activations: (limit?: number) =>
    invoke<[string, string, Record<string, unknown>, number][]>("credits_activations", { limit }),
  saveSettings: (settings: ProxySettings) =>
    invoke<ProxySettings>("credits_save_settings", { settings }),
  startProxy: () => invoke<GatewayStatus>("credits_start_proxy"),
  stopProxy: () => invoke<GatewayStatus>("credits_stop_proxy"),
  proxyKey: () => invoke<string>("credits_proxy_key"),
  probeProxy: () => invoke<string>("credits_probe_proxy"),
  modelHealth: () => invoke<ModelHealthReport[]>("credits_model_health"),
  verifyModel: (accountId: string, modelId: string) =>
    invoke<CreditModel>("credits_verify_model", { accountId, modelId }),
  oauthStart: (platform: CreditPlatform, region: CreditRegion = "cn") => invoke<OAuthStart>("credits_oauth_start", { platform, region }),
  oauthPoll: (flowId: string) => invoke<OAuthResult>("credits_oauth_poll", { flowId }),
};
