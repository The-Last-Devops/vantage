{{- define "lm.name" -}}{{ .Release.Name }}{{- end -}}
{{- define "lm.db" -}}{{ .Release.Name }}-db{{- end -}}
{{- define "lm.hub" -}}{{ .Release.Name }}-hub{{- end -}}

{{- define "lm.configUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://lastmon:{{ .Values.timescaledb.password }}@{{ include "lm.db" . }}:5432/lastmon_config
{{- else -}}{{ .Values.hub.configDatabaseUrl }}{{- end -}}
{{- end -}}

{{- define "lm.dataUrl" -}}
{{- if .Values.timescaledb.enabled -}}
postgres://lastmon:{{ .Values.timescaledb.password }}@{{ include "lm.db" . }}:5432/lastmon_data
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
