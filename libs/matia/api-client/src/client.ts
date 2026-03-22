import { CatalogApi } from './catalog';
import { ConnectorApi } from './connector';
import { HttpClient } from './http';
import { IssueApi } from './issue';
import { PipelineApi } from './pipeline';
import { QualityApi } from './quality';

export interface MatiaClientOptions {
  /** Base URL of the Matia API. Defaults to '/api' for same-origin. */
  baseUrl?: string;
}

/**
 * Matia data platform API client.
 *
 * @example
 * ```ts
 * import { MatiaClient } from '@matia/api-client';
 *
 * const matia = new MatiaClient({ baseUrl: 'http://localhost:8080' });
 *
 * const datasets = await matia.catalog.list();
 * const report = await matia.quality.report('sales');
 * const pipeline = await matia.pipelines.run('revenue_by_category');
 * ```
 */
export class MatiaClient {
  private http: HttpClient;

  readonly catalog: CatalogApi;
  readonly pipelines: PipelineApi;
  readonly quality: QualityApi;
  readonly connectors: ConnectorApi;
  readonly issues: IssueApi;

  constructor(options: MatiaClientOptions = {}) {
    this.http = new HttpClient(options.baseUrl ?? '');
    this.catalog = new CatalogApi(this.http);
    this.pipelines = new PipelineApi(this.http);
    this.quality = new QualityApi(this.http);
    this.connectors = new ConnectorApi(this.http);
    this.issues = new IssueApi(this.http);
  }
}
