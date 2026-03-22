/** Connector category. */
export type ConnectorType =
  | 'database'
  | 'messaging'
  | 'storage'
  | 'vector'
  | 'timeseries'
  | 'warehouse';

/** Data flow direction. */
export type ConnectorDirection = 'source' | 'sink' | 'both';

export type ConnectorStatus = 'connected' | 'disconnected' | 'error';

/** Use case category for connector catalog. */
export type ConnectorUseCase =
  | 'databases'
  | 'engineering'
  | 'human_resources'
  | 'finance'
  | 'files'
  | 'marketing'
  | 'sales_support'
  | 'warehouse_datalakes'
  | 'others';

/** A catalog entry for a connector (available connectors). */
export interface ConnectorCatalogEntry {
  name: string;
  slug: string;
  direction: ConnectorDirection;
  useCase: ConnectorUseCase;
}

/** A configured integration connector. */
export interface Connector {
  id: string;
  name: string;
  type: ConnectorType;
  direction: ConnectorDirection;
  status: ConnectorStatus;
  config: Record<string, string>;
  datasets: number;
  created_at: string;
  last_checked: string | null;
}

/** Request to register a new connector. */
export interface CreateConnectorRequest {
  name: string;
  type: ConnectorType;
  direction: ConnectorDirection;
  config: Record<string, string>;
}

/** Supported connector templates. */
export const CONNECTOR_TEMPLATES = {
  postgres: { type: 'database' as const, direction: 'both' as const, config_keys: ['host', 'port', 'user', 'password', 'database'] },
  mongo: { type: 'database' as const, direction: 'both' as const, config_keys: ['url', 'database'] },
  redis: { type: 'database' as const, direction: 'source' as const, config_keys: ['url'] },
  nats: { type: 'messaging' as const, direction: 'both' as const, config_keys: ['url'] },
  qdrant: { type: 'vector' as const, direction: 'both' as const, config_keys: ['url', 'api_key'] },
  influxdb: { type: 'timeseries' as const, direction: 'source' as const, config_keys: ['url', 'token', 'org', 'bucket'] },
  bigquery: { type: 'warehouse' as const, direction: 'sink' as const, config_keys: ['project', 'dataset', 'credentials_json'] },
  bigtable: { type: 'warehouse' as const, direction: 'sink' as const, config_keys: ['project', 'instance', 'credentials_json'] },
  s3: { type: 'storage' as const, direction: 'sink' as const, config_keys: ['bucket', 'region', 'access_key', 'secret_key'] },
} as const;

/** Use case display labels. */
export const USE_CASE_LABELS: Record<ConnectorUseCase, string> = {
  databases: 'Databases',
  engineering: 'Engineering',
  human_resources: 'Human Resources',
  finance: 'Finance',
  files: 'Files',
  marketing: 'Marketing',
  sales_support: 'Sales & Support Ops',
  warehouse_datalakes: 'Warehouse & Datalakes',
  others: 'Others',
};

/** Full connector catalog matching Matia's connector list. */
export const CONNECTOR_CATALOG: ConnectorCatalogEntry[] = [
  // --- Sources + Destinations (both) ---
  { name: 'AWS Redshift', slug: 'aws-redshift', direction: 'both', useCase: 'warehouse_datalakes' },
  { name: 'AWS S3', slug: 'aws-s3', direction: 'both', useCase: 'files' },
  { name: 'BigQuery', slug: 'bigquery', direction: 'both', useCase: 'warehouse_datalakes' },
  { name: 'Customer.io', slug: 'customer-io', direction: 'both', useCase: 'marketing' },
  { name: 'Databricks', slug: 'databricks', direction: 'both', useCase: 'warehouse_datalakes' },
  { name: 'Postgres', slug: 'postgres', direction: 'both', useCase: 'databases' },
  { name: 'Salesforce', slug: 'salesforce', direction: 'both', useCase: 'sales_support' },
  { name: 'Snowflake', slug: 'snowflake', direction: 'both', useCase: 'warehouse_datalakes' },

  // --- Destination only ---
  { name: 'Gainsight', slug: 'gainsight', direction: 'sink', useCase: 'sales_support' },

  // --- Source only ---
  { name: 'Ada', slug: 'ada', direction: 'source', useCase: 'sales_support' },
  { name: 'Aircall', slug: 'aircall', direction: 'source', useCase: 'sales_support' },
  { name: 'Airtable', slug: 'airtable', direction: 'source', useCase: 'engineering' },
  { name: 'Algolia', slug: 'algolia', direction: 'source', useCase: 'engineering' },
  { name: 'Amazon Ads', slug: 'amazon-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Amazon Selling Partner', slug: 'amazon-selling-partner', direction: 'source', useCase: 'sales_support' },
  { name: 'Amplitude', slug: 'amplitude', direction: 'source', useCase: 'engineering' },
  { name: 'App Lovin', slug: 'app-lovin', direction: 'source', useCase: 'marketing' },
  { name: 'Apple App Store', slug: 'apple-app-store', direction: 'source', useCase: 'engineering' },
  { name: 'Apple Search Ads', slug: 'apple-search-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Appsflyer', slug: 'appsflyer', direction: 'source', useCase: 'marketing' },
  { name: 'Asana', slug: 'asana', direction: 'source', useCase: 'engineering' },
  { name: 'Ashby', slug: 'ashby', direction: 'source', useCase: 'human_resources' },
  { name: 'Attentive', slug: 'attentive', direction: 'source', useCase: 'marketing' },
  { name: 'Avalara', slug: 'avalara', direction: 'source', useCase: 'finance' },
  { name: 'Braintree', slug: 'braintree', direction: 'source', useCase: 'finance' },
  { name: 'Braze', slug: 'braze', direction: 'source', useCase: 'marketing' },
  { name: 'Calendly', slug: 'calendly', direction: 'source', useCase: 'sales_support' },
  { name: 'Cassandra', slug: 'cassandra', direction: 'source', useCase: 'databases' },
  { name: 'Catalyst', slug: 'catalyst', direction: 'source', useCase: 'sales_support' },
  { name: 'Chameleon', slug: 'chameleon', direction: 'source', useCase: 'engineering' },
  { name: 'Cheddar', slug: 'cheddar', direction: 'source', useCase: 'finance' },
  { name: 'Clari', slug: 'clari', direction: 'source', useCase: 'sales_support' },
  { name: 'ClickUp', slug: 'clickup', direction: 'source', useCase: 'engineering' },
  { name: 'Datadog', slug: 'datadog', direction: 'source', useCase: 'engineering' },
  { name: 'Dialpad', slug: 'dialpad', direction: 'source', useCase: 'sales_support' },
  { name: 'Dwolla', slug: 'dwolla', direction: 'source', useCase: 'finance' },
  { name: 'DynamoDB', slug: 'dynamodb', direction: 'source', useCase: 'databases' },
  { name: 'Elastic', slug: 'elastic', direction: 'source', useCase: 'databases' },
  { name: 'Email', slug: 'email', direction: 'source', useCase: 'others' },
  { name: 'Facebook Ads', slug: 'facebook-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Facebook Pages', slug: 'facebook-pages', direction: 'source', useCase: 'marketing' },
  { name: 'Fibery', slug: 'fibery', direction: 'source', useCase: 'engineering' },
  { name: 'Firebase', slug: 'firebase', direction: 'source', useCase: 'databases' },
  { name: 'Front', slug: 'front', direction: 'source', useCase: 'sales_support' },
  { name: 'FullStory', slug: 'fullstory', direction: 'source', useCase: 'engineering' },
  { name: 'GitLab', slug: 'gitlab', direction: 'source', useCase: 'engineering' },
  { name: 'Github', slug: 'github', direction: 'source', useCase: 'engineering' },
  { name: 'Gong', slug: 'gong', direction: 'source', useCase: 'sales_support' },
  { name: 'Google Ads', slug: 'google-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Google Analytics', slug: 'google-analytics', direction: 'source', useCase: 'marketing' },
  { name: 'Google Analytics Export 4', slug: 'google-analytics-export-4', direction: 'source', useCase: 'marketing' },
  { name: 'Google Drive', slug: 'google-drive', direction: 'source', useCase: 'files' },
  { name: 'Google Play', slug: 'google-play', direction: 'source', useCase: 'engineering' },
  { name: 'Google Search Console', slug: 'google-search-console', direction: 'source', useCase: 'marketing' },
  { name: 'Google Sheets', slug: 'google-sheets', direction: 'source', useCase: 'files' },
  { name: 'Gorgias', slug: 'gorgias', direction: 'source', useCase: 'sales_support' },
  { name: 'Greenhouse', slug: 'greenhouse', direction: 'source', useCase: 'human_resources' },
  { name: 'Harvest', slug: 'harvest', direction: 'source', useCase: 'finance' },
  { name: 'HubSpot', slug: 'hubspot', direction: 'source', useCase: 'marketing' },
  { name: 'Impact', slug: 'impact', direction: 'source', useCase: 'marketing' },
  { name: 'Intercom', slug: 'intercom', direction: 'source', useCase: 'sales_support' },
  { name: 'Ironclad', slug: 'ironclad', direction: 'source', useCase: 'others' },
  { name: 'Iterable', slug: 'iterable', direction: 'source', useCase: 'marketing' },
  { name: 'Jira', slug: 'jira', direction: 'source', useCase: 'engineering' },
  { name: 'Klaviyo', slug: 'klaviyo', direction: 'source', useCase: 'marketing' },
  { name: 'Launch Darkly', slug: 'launch-darkly', direction: 'source', useCase: 'engineering' },
  { name: 'Linear', slug: 'linear', direction: 'source', useCase: 'engineering' },
  { name: 'Linkedin Ads', slug: 'linkedin-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Loop', slug: 'loop', direction: 'source', useCase: 'sales_support' },
  { name: 'MSSQL', slug: 'mssql', direction: 'source', useCase: 'databases' },
  { name: 'MaestroQA', slug: 'maestroqa', direction: 'source', useCase: 'sales_support' },
  { name: 'Mailchimp', slug: 'mailchimp', direction: 'source', useCase: 'marketing' },
  { name: 'Microsoft Ads', slug: 'microsoft-ads', direction: 'source', useCase: 'marketing' },
  { name: 'Mixpanel', slug: 'mixpanel', direction: 'source', useCase: 'engineering' },
  { name: 'Monday', slug: 'monday', direction: 'source', useCase: 'engineering' },
  { name: 'MongoDB', slug: 'mongodb', direction: 'source', useCase: 'databases' },
  { name: 'MySQL', slug: 'mysql', direction: 'source', useCase: 'databases' },
  { name: 'Netsuite', slug: 'netsuite', direction: 'source', useCase: 'finance' },
  { name: 'Notion', slug: 'notion', direction: 'source', useCase: 'engineering' },
  { name: 'Okta', slug: 'okta', direction: 'source', useCase: 'engineering' },
  { name: 'OpsGenie', slug: 'opsgenie', direction: 'source', useCase: 'engineering' },
  { name: 'Outreach', slug: 'outreach', direction: 'source', useCase: 'sales_support' },
  { name: 'PagerDuty', slug: 'pagerduty', direction: 'source', useCase: 'engineering' },
  { name: 'Pendo', slug: 'pendo', direction: 'source', useCase: 'engineering' },
  { name: 'Pinterest', slug: 'pinterest', direction: 'source', useCase: 'marketing' },
  { name: 'PostHog', slug: 'posthog', direction: 'source', useCase: 'engineering' },
  { name: 'Productboard', slug: 'productboard', direction: 'source', useCase: 'engineering' },
  { name: 'Qualtrics', slug: 'qualtrics', direction: 'source', useCase: 'others' },
  { name: 'Quickbooks', slug: 'quickbooks', direction: 'source', useCase: 'finance' },
  { name: 'Ramp', slug: 'ramp', direction: 'source', useCase: 'finance' },
  { name: 'Recharge', slug: 'recharge', direction: 'source', useCase: 'finance' },
  { name: 'Recurly', slug: 'recurly', direction: 'source', useCase: 'finance' },
  { name: 'Reply.io', slug: 'reply-io', direction: 'source', useCase: 'sales_support' },
  { name: 'SFTP', slug: 'sftp', direction: 'source', useCase: 'files' },
  { name: 'STAT Search Analytics', slug: 'stat-search-analytics', direction: 'source', useCase: 'marketing' },
  { name: 'Sakari', slug: 'sakari', direction: 'source', useCase: 'sales_support' },
  { name: 'Salesloft', slug: 'salesloft', direction: 'source', useCase: 'sales_support' },
  { name: 'Samsara', slug: 'samsara', direction: 'source', useCase: 'others' },
  { name: 'Sendgrid', slug: 'sendgrid', direction: 'source', useCase: 'marketing' },
  { name: 'ServiceNow', slug: 'servicenow', direction: 'source', useCase: 'engineering' },
  { name: 'Shipbob', slug: 'shipbob', direction: 'source', useCase: 'others' },
  { name: 'Shopify', slug: 'shopify', direction: 'source', useCase: 'sales_support' },
  { name: 'Slack', slug: 'slack', direction: 'source', useCase: 'engineering' },
  { name: 'Snapchat', slug: 'snapchat', direction: 'source', useCase: 'marketing' },
  { name: 'Snowplow', slug: 'snowplow', direction: 'source', useCase: 'engineering' },
  { name: 'Statuspage', slug: 'statuspage', direction: 'source', useCase: 'engineering' },
  { name: 'Stripe', slug: 'stripe', direction: 'source', useCase: 'finance' },
  { name: 'SurveyMonkey', slug: 'surveymonkey', direction: 'source', useCase: 'others' },
  { name: 'Tik Tok Shop', slug: 'tiktok-shop', direction: 'source', useCase: 'sales_support' },
  { name: 'TikTok', slug: 'tiktok', direction: 'source', useCase: 'marketing' },
  { name: 'Twilio', slug: 'twilio', direction: 'source', useCase: 'sales_support' },
  { name: 'Typeform', slug: 'typeform', direction: 'source', useCase: 'others' },
  { name: 'Vitally', slug: 'vitally', direction: 'source', useCase: 'sales_support' },
  { name: 'Webhook', slug: 'webhook', direction: 'source', useCase: 'others' },
  { name: 'Workday Reports', slug: 'workday-reports', direction: 'source', useCase: 'human_resources' },
  { name: 'Xero', slug: 'xero', direction: 'source', useCase: 'finance' },
  { name: 'Zendesk', slug: 'zendesk', direction: 'source', useCase: 'sales_support' },
  { name: 'Zoho', slug: 'zoho', direction: 'source', useCase: 'sales_support' },
  { name: 'Zoom', slug: 'zoom', direction: 'source', useCase: 'others' },
];
