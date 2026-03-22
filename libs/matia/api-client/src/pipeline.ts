import type { CreatePipelineRequest, Pipeline, PipelineRunResult } from '@matia/types';
import type { HttpClient } from './http';

export class PipelineApi {
  constructor(private http: HttpClient) {}

  list(): Promise<Pipeline[]> {
    return this.http.get('/api/analytics/pipelines');
  }

  get(id: string): Promise<Pipeline> {
    return this.http.get(`/api/analytics/pipelines/${id}`);
  }

  create(request: CreatePipelineRequest): Promise<Pipeline> {
    return this.http.post('/api/analytics/pipelines', request);
  }

  run(id: string): Promise<PipelineRunResult> {
    return this.http.post(`/api/analytics/pipelines/${id}/run`);
  }

  status(id: string): Promise<PipelineRunResult> {
    return this.http.get(`/api/analytics/pipelines/${id}/status`);
  }

  result(id: string): Promise<Record<string, unknown>[]> {
    return this.http.get(`/api/analytics/pipelines/${id}/result`);
  }

  delete(id: string): Promise<void> {
    return this.http.delete(`/api/analytics/pipelines/${id}`);
  }
}
