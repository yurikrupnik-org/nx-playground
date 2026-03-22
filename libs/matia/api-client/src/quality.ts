import type { ConfigureQualityRequest, QualityReport } from '@matia/types';
import type { HttpClient } from './http';

export class QualityApi {
  constructor(private http: HttpClient) {}

  report(dataset: string): Promise<QualityReport> {
    return this.http.get(`/api/analytics/quality/${dataset}`);
  }

  configure(request: ConfigureQualityRequest): Promise<void> {
    return this.http.post('/api/analytics/quality', request);
  }
}
