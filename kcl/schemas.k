schema GcpBinding:
    gsaEmail: str
    providerSuffix?: str

schema AwsBinding:
    roleArn: str
    region: str
    providerSuffix?: str

schema CloudBindings:
    gcp?: GcpBinding
    aws?: AwsBinding

schema Workload:
    name: str
    namespace: str
    ksa: str
    clouds: CloudBindings
