import type { Connector, CreateConnectorRequest } from '@matia/types';
import type { HttpClient } from './http';

export class ConnectorApi {
  constructor(private http: HttpClient) {}

  list(): Promise<Connector[]> {
    return this.http.get('/api/analytics/connectors');
  }

  get(id: string): Promise<Connector> {
    return this.http.get(`/api/analytics/connectors/${id}`);
  }

  create(request: CreateConnectorRequest): Promise<Connector> {
    return this.http.post('/api/analytics/connectors', request);
  }

  test(id: string): Promise<{ status: string; error?: string }> {
    return this.http.post(`/api/analytics/connectors/${id}/test`);
  }

  delete(id: string): Promise<void> {
    return this.http.delete(`/api/analytics/connectors/${id}`);
  }
}
