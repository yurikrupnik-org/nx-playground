import type { DatasetMeta, RegisterDatasetRequest } from '@matia/types';
import type { HttpClient } from './http';

export class CatalogApi {
  constructor(private http: HttpClient) {}

  list(): Promise<DatasetMeta[]> {
    return this.http.get('/api/analytics/catalog');
  }

  get(name: string): Promise<DatasetMeta> {
    return this.http.get(`/api/analytics/catalog/${name}`);
  }

  sample(name: string, limit = 100): Promise<Record<string, unknown>[]> {
    return this.http.get(`/api/analytics/catalog/${name}/sample?limit=${limit}`);
  }

  stats(name: string): Promise<Record<string, unknown>> {
    return this.http.get(`/api/analytics/catalog/${name}/stats`);
  }

  register(request: RegisterDatasetRequest): Promise<DatasetMeta> {
    return this.http.post('/api/analytics/catalog', request);
  }

  delete(name: string): Promise<void> {
    return this.http.delete(`/api/analytics/catalog/${name}`);
  }

  lineage(name: string): Promise<string[]> {
    return this.http.get(`/api/analytics/lineage/${name}`);
  }
}
