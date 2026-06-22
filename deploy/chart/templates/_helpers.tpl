{{- define "lm.name" -}}{{ .Release.Name }}{{- end -}}
{{- define "lm.db" -}}{{ .Release.Name }}-db{{- end -}}
{{- define "lm.hub" -}}{{ .Release.Name }}-hub{{- end -}}

{{- /*
  In-cluster DB password: use the value if set, else reuse the one already stored
  in the Secret (so `helm upgrade` keeps it), else generate a random one. Memoized
  into .Values so every call in this render returns the same value.
*/ -}}
{{- define "lm.dbPassword" -}}
{{- if not .Values.timescaledb._pw -}}
  {{- $pw := .Values.timescaledb.password -}}
  {{- if not $pw -}}
    {{- $existing := lookup "v1" "Secret" .Release.Namespace (include "lm.name" .) -}}
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

{{- define "lm.configUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://lastmon:{{ include "lm.dbPassword" . }}@{{ include "lm.db" . }}:5432/lastmon_config
{{- else -}}{{ .Values.hub.configDatabaseUrl }}{{- end -}}
{{- end -}}

{{- define "lm.dataUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://lastmon:{{ include "lm.dbPassword" . }}@{{ include "lm.db" . }}:5432/lastmon_data
{{- else -}}{{ .Values.hub.dataDatabaseUrl }}{{- end -}}
{{- end -}}

{{- define "lm.pullSecrets" -}}
{{- with .Values.image.pullSecrets }}
imagePullSecrets:
{{- range . }}
  - name: {{ . }}
{{- end }}
{{- end }}
{{- end -}}
