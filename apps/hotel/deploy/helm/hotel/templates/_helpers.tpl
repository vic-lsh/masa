{{- define "hotel.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "hotel.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "hotel.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name (include "hotel.name" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "hotel.labels" -}}
app.kubernetes.io/name: {{ include "hotel.name" .context }}
helm.sh/chart: {{ include "hotel.chart" .context }}
app.kubernetes.io/instance: {{ .context.Release.Name }}
app.kubernetes.io/managed-by: {{ .context.Release.Service }}
{{- if .component }}
app.kubernetes.io/component: {{ .component }}
{{- end }}
{{- end -}}

{{- define "hotel.selectorLabels" -}}
app.kubernetes.io/name: {{ include "hotel.name" .context }}
app.kubernetes.io/instance: {{ .context.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "hotel.componentName" -}}
{{- printf "%s-%s" (include "hotel.fullname" .context) .name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "hotel.componentHostname" -}}
{{- include "hotel.componentName" . -}}
{{- end -}}

{{- define "hotel.image" -}}
{{- $ctx := .context -}}
{{- $img := .image -}}
{{- $registry := default $ctx.Values.global.image.registry $img.registry -}}
{{- $repository := default "" $img.repository -}}
{{- $tag := default $ctx.Values.global.image.tag $img.tag -}}
{{- if not $repository -}}
{{- fail "repository must be set for each image" -}}
{{- end -}}
{{- if $registry -}}
{{- printf "%s/%s:%s" $registry $repository $tag -}}
{{- else -}}
{{- printf "%s:%s" $repository $tag -}}
{{- end -}}
{{- end -}}

{{- define "hotel.pullPolicy" -}}
{{- $ctx := .context -}}
{{- $img := .image -}}
{{- default $ctx.Values.global.image.pullPolicy $img.pullPolicy -}}
{{- end -}}

{{- define "hotel.configMapName" -}}
{{- printf "%s-config" (include "hotel.fullname" .) -}}
{{- end -}}

{{- define "hotel.serviceAccountName" -}}
{{- if .Values.serviceAccount.create -}}
  {{- if .Values.serviceAccount.name -}}
    {{- .Values.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
  {{- else -}}
    {{- include "hotel.fullname" . -}}
  {{- end -}}
{{- else -}}
  {{- if .Values.serviceAccount.name -}}
    {{- .Values.serviceAccount.name | trunc 63 | trimSuffix "-" -}}
  {{- else -}}
    default
  {{- end -}}
{{- end -}}
{{- end -}}

{{- define "hotel.serviceAnnotations" -}}
{{- if .annotations -}}
{{- toYaml .annotations -}}
{{- end -}}
{{- end -}}
