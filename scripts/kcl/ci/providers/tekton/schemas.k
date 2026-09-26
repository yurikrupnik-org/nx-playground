"""
Tekton CRD schemas.
The tekton KCL package only has PipelineRun/TaskRun, so we define Task/Pipeline here.
"""

import k8s.apimachinery.pkg.apis.meta.v1 as metav1

# =============================================================================
# TEKTON TASK SCHEMAS
# =============================================================================
schema TektonTask:
    """Tekton Task resource"""
    apiVersion: str = "tekton.dev/v1"
    kind: str = "Task"
    metadata: metav1.ObjectMeta
    spec: TektonTaskSpec

schema TektonTaskSpec:
    description?: str
    params?: [TektonParam]
    steps: [TektonStep]
    workspaces?: [TektonWorkspaceDeclaration]
    results?: [TektonResult]

schema TektonParam:
    name: str
    $type?: str = "string"
    description?: str
    default?: str

schema TektonStep:
    name: str
    image: str
    script?: str
    command?: [str]
    args?: [str]
    env?: [TektonEnvVar]
    workingDir?: str

schema TektonEnvVar:
    name: str
    value?: str
    valueFrom?: any

schema TektonWorkspaceDeclaration:
    name: str
    description?: str
    mountPath?: str

schema TektonResult:
    name: str
    description?: str

# =============================================================================
# TEKTON PIPELINE SCHEMAS
# =============================================================================
schema TektonPipeline:
    """Tekton Pipeline resource"""
    apiVersion: str = "tekton.dev/v1"
    kind: str = "Pipeline"
    metadata: metav1.ObjectMeta
    spec: TektonPipelineSpec

schema TektonPipelineSpec:
    description?: str
    params?: [TektonParam]
    tasks: [TektonPipelineTask]
    workspaces?: [TektonPipelineWorkspace]
    $finally?: [TektonPipelineTask]

schema TektonPipelineTask:
    name: str
    taskRef?: TektonTaskRef
    taskSpec?: TektonTaskSpec
    runAfter?: [str]
    params?: [TektonPipelineTaskParam]
    workspaces?: [TektonPipelineTaskWorkspace]
    when?: [TektonWhenExpression]

schema TektonTaskRef:
    name: str
    kind?: str = "Task"

schema TektonPipelineTaskParam:
    name: str
    value: str

schema TektonPipelineTaskWorkspace:
    name: str
    workspace: str

schema TektonPipelineWorkspace:
    name: str

schema TektonWhenExpression:
    input: str
    operator: "in" | "notin"
    values: [str]
