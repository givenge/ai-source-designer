export type TaskStatus =
  | 'draft'
  | 'sampling'
  | 'synthesizing'
  | 'validating'
  | 'repairing'
  | 'completed'
  | 'failed';

export interface AiSettings {
  baseUrl: string;
  model: string;
  apiKey: string;
}

export interface TaskDraft {
  targetUrl: string;
  testKeyword?: string | null;
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
}

export interface TocRule {
  init?: string | null;
  chapterList?: string | null;
  chapterName?: string | null;
  chapterUrl?: string | null;
  nextTocUrl?: string | null;
}

export interface ContentRule {
  content?: string | null;
  nextContentUrl?: string | null;
  replaceRegex?: string | null;
}

export interface BookSource {
  bookSourceName: string;
  bookSourceUrl: string;
  enabled?: boolean | null;
  searchUrl?: string | null;
  ruleSearch?: BookListRule | null;
  ruleBookInfo?: BookInfoRule | null;
  ruleToc?: TocRule | null;
  ruleContent?: ContentRule | null;
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
