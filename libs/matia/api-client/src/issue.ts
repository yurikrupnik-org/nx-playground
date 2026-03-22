import type { Issue, IssueStatus } from '@matia/types';
import type { HttpClient } from './http';

export class IssueApi {
  constructor(private http: HttpClient) {}

  list(status?: IssueStatus): Promise<Issue[]> {
    const qs = status ? `?status=${status}` : '';
    return this.http.get(`/api/analytics/issues${qs}`);
  }

  get(id: string): Promise<Issue> {
    return this.http.get(`/api/analytics/issues/${id}`);
  }

  acknowledge(id: string): Promise<Issue> {
    return this.http.put(`/api/analytics/issues/${id}`, { status: 'acknowledged' });
  }

  resolve(id: string): Promise<Issue> {
    return this.http.put(`/api/analytics/issues/${id}`, { status: 'resolved' });
  }
}
