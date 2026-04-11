export type TaskStatus =
  | 'draft'
  | 'sampling'
  | 'synthesizing'
  | 'validating'
  | 'repairing'
  | 'completed'
  | 'failed';

export type TaskMode = 'classic' | 'agent';

export interface AiSettings {
  baseUrl: string;
  model: string;
  apiKey: string;
}

export interface TaskDraft {
  targetUrl: string;
  testKeyword?: string | null;
  taskMode?: TaskMode;
}

export interface EvidenceRef {
  label: string;
  artifactPath: string;
  detail?: string | null;
}

export interface ArtifactManifest {
  screenshots: string[];
  dom: string[];
  network: string[];
  logs: string[];
  output: string[];
}

export interface RepairStep {
  label: string;
  reason: string;
  appliedAt: string;
}

export interface BookListRule {
  checkKeyWord?: string | null;
  bookList?: string | null;
  name?: string | null;
  author?: string | null;
  intro?: string | null;
  kind?: string | null;
  lastChapter?: string | null;
  updateTime?: string | null;
  bookUrl?: string | null;
  coverUrl?: string | null;
  wordCount?: string | null;
}

export type SearchRule = BookListRule;
export type ExploreRule = BookListRule;

export interface BookInfoRule {
  init?: string | null;
  name?: string | null;
  author?: string | null;
  intro?: string | null;
  kind?: string | null;
  lastChapter?: string | null;
  updateTime?: string | null;
  coverUrl?: string | null;
  wordCount?: string | null;
  tocUrl?: string | null;
  canReName?: string | null;
  downloadUrls?: string | null;
}

export interface TocRule {
  preUpdateJs?: string | null;
  init?: string | null;
  chapterList?: string | null;
  chapterName?: string | null;
  chapterUrl?: string | null;
  formatJs?: string | null;
  isVolume?: string | null;
  isVip?: string | null;
  isPay?: string | null;
  updateTime?: string | null;
  nextTocUrl?: string | null;
}

export interface ContentRule {
  content?: string | null;
  title?: string | null;
  nextContentUrl?: string | null;
  webJs?: string | null;
  sourceRegex?: string | null;
  replaceRegex?: string | null;
  imageStyle?: string | null;
  imageDecode?: string | null;
  payAction?: string | null;
}

export interface BookSource {
  bookSourceName: string;
  bookSourceUrl: string;
  bookSourceGroup?: string | null;
  bookSourceType?: number | null;
  bookUrlPattern?: string | null;
  customOrder?: number | null;
  enabled?: boolean | null;
  enabledExplore?: boolean | null;
  enabledCookieJar?: boolean | null;
  jsLib?: string | null;
  concurrentRate?: string | null;
  header?: unknown;
  loginUrl?: string | null;
  loginUi?: unknown;
  loginCheckJs?: string | null;
  coverDecodeJs?: string | null;
  lastUpdateTime?: number | null;
  weight?: number | null;
  exploreUrl?: string | null;
  exploreScreen?: unknown;
  ruleExplore?: ExploreRule | null;
  searchUrl?: string | null;
  ruleSearch?: SearchRule | null;
  ruleBookInfo?: BookInfoRule | null;
  ruleToc?: TocRule | null;
  ruleContent?: ContentRule | null;
  ruleReview?: unknown;
  bookSourceComment?: string | null;
  variableComment?: string | null;
  respondTime?: number | null;
  loadWithBaseUrl?: boolean | null;
  singleUrl?: boolean | null;
}

export interface SourceCandidate {
  strategy: string;
  confidence: number;
  evidenceRefs: EvidenceRef[];
  repairHistory: RepairStep[];
  bookSource: BookSource;
}

export interface ValidationPhase {
  passed: boolean;
  summary: string;
  details: string[];
  evidenceRefs: EvidenceRef[];
}

export interface ValidationReport {
  explore: ValidationPhase;
  search: ValidationPhase;
  bookInfo: ValidationPhase;
  toc: ValidationPhase;
  content: ValidationPhase;
  cleanup: ValidationPhase;
}

export interface LogEntry {
  at: string;
  stage: TaskStatus;
  message: string;
}

export interface TaskRecord {
  id: string;
  status: TaskStatus;
  taskMode: TaskMode;
  createdAt: string;
  updatedAt: string;
  targetUrl: string;
  testKeyword?: string | null;
  errorSummary?: string | null;
  artifactManifest?: ArtifactManifest | null;
  sourceCandidate?: SourceCandidate | null;
  validationReport?: ValidationReport | null;
  logs: LogEntry[];
}

export interface ArtifactFile {
  path: string;
  label: string;
  contentType: string;
  contents: string;
}

export interface PageLink {
  label: string;
  url: string;
}

export interface PageForm {
  action: string;
  method: string;
  inputNames: string[];
}

export interface PageInspection {
  page: {
    role: string;
    url: string;
    htmlPath: string;
    screenshotPath?: string | null;
    snippetPaths: string[];
  };
  title: string;
  excerpt: string;
  links: PageLink[];
  forms: PageForm[];
  hints: {
    bookListCandidates: string[];
    titleLinkCandidates: string[];
    tocLinkCandidates: string[];
    contentCandidates: string[];
    nextPageCandidates: string[];
  };
  networkSamples: {
    label: string;
    requestUrl: string;
    method: string;
    responsePath: string;
    notes?: string | null;
  }[];
  manifest: ArtifactManifest;
}

export interface ArtifactBundle {
  manifest: ArtifactManifest;
  screenshots: ArtifactFile[];
  dom: ArtifactFile[];
  network: ArtifactFile[];
  logs: ArtifactFile[];
  output: ArtifactFile[];
}

export interface StartTaskPayload {
  taskId: string;
  aiSettings: {
    baseUrl: string;
    model: string;
  };
  apiKey?: string | null;
}

export interface ChatRepairTaskPayload {
  taskId: string;
  message: string;
  aiSettings: {
    baseUrl: string;
    model: string;
  };
  apiKey?: string | null;
}

export interface AgentChatRepairResult {
  reply: string;
  applied: boolean;
  modelRawOutput?: string | null;
  task: TaskRecord;
}
