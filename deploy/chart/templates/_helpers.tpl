{{- define "vantage.name" -}}{{ .Release.Name }}{{- end -}}
{{- define "vantage.dbConfig" -}}{{ .Release.Name }}-db-config{{- end -}}
{{- define "vantage.dbData" -}}{{ .Release.Name }}-db-data{{- end -}}
{{- define "vantage.hub" -}}{{ .Release.Name }}-hub{{- end -}}

{{- /*
  In-cluster DB password: use the value if set, else reuse the one already stored
  in the Secret (so `helm upgrade` keeps it), else generate a random one. Memoized
  into .Values so every call in this render returns the same value.
*/ -}}
{{- define "vantage.dbPassword" -}}
{{- if not .Values.timescaledb._pw -}}
  {{- $pw := .Values.timescaledb.password -}}
  {{- if not $pw -}}
    {{- $existing := lookup "v1" "Secret" .Release.Namespace (include "vantage.name" .) -}}
    {{- if and $existing $existing.data (hasKey $existing.data "db-password") -}}
      {{- $pw = index $existing.data "db-password" | b64dec -}}
    {{- else -}}
      {{- $pw = randAlphaNum 24 -}}
    {{- end -}}
  {{- end -}}
  {{- $_ := set .Values.timescaledb "_pw" $pw -}}
{{- end -}}
{{- .Values.timescaledb._pw -}}
{{- end -}}

{{- define "vantage.configUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://vantage:{{ include "vantage.dbPassword" . }}@{{ include "vantage.dbConfig" . }}:5432/vantage_config
{{- else -}}{{ .Values.hub.configDatabaseUrl }}{{- end -}}
{{- end -}}

{{- define "vantage.dataUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://vantage:{{ include "vantage.dbPassword" . }}@{{ include "vantage.dbData" . }}:5432/vantage_data
{{- else -}}{{ .Values.hub.dataDatabaseUrl }}{{- end -}}
{{- end -}}

{{- /*
  EXEC_APP_SECRET: explicit hub.appSecret wins; else if hub.autoAppSecret, reuse the
  one already in the Secret (stable across upgrades) or generate one. Returns "" when
  neither is set. Memoized so every call in this render agrees.
*/ -}}
{{- define "vantage.appSecret" -}}
{{- if not (hasKey .Values.hub "_appSecret") -}}
  {{- $s := .Values.hub.appSecret -}}
  {{- if and (not $s) .Values.hub.autoAppSecret -}}
    {{- $existing := lookup "v1" "Secret" .Release.Namespace (include "vantage.name" .) -}}
    {{- if and $existing $existing.data (hasKey $existing.data "app-secret") -}}
      {{- $s = index $existing.data "app-secret" | b64dec -}}
    {{- else -}}
      {{- $s = randAlphaNum 32 -}}
    {{- end -}}
  {{- end -}}
  {{- $_ := set .Values.hub "_appSecret" $s -}}
{{- end -}}
{{- .Values.hub._appSecret -}}
{{- end -}}

{{- /* Whether an app secret will be provisioned at all. */ -}}
{{- define "vantage.appSecretEnabled" -}}
{{- if or .Values.hub.appSecret .Values.hub.autoAppSecret -}}true{{- end -}}
{{- end -}}

{{- /* URL scheme for passkey RP / PUBLIC_URL — https only when TLS is terminated. */ -}}
{{- define "vantage.scheme" -}}{{ ternary "https" "http" .Values.hub.ingress.tls }}{{- end -}}

{{- define "vantage.pullSecrets" -}}
{{- with .Values.image.pullSecrets }}
imagePullSecrets:
{{- range . }}
  - name: {{ . }}
{{- end }}
{{- end }}
{{- end -}}
