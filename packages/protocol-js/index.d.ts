export interface BuildTimeRenderingStreamRepeat {
  type: 'repeat'
  value: string
  template: string
}

export interface BuildTimeRenderingStreamAttribute {
  type: 'attribute'
  key: string
  value: string
}

export interface BuildTimeRenderingStreamRaw {
  type: 'raw'
  value: string
}

export interface BuildTimeRenderingStreamSignal {
  type: 'signal'
  value: string
  defaultValue?: string
}

export interface BuildTimeRenderingStreamWhen {
  type: 'when'
  value: string
}

export interface BuildTimeRenderingStreamComponent {
  type: 'component'
  value: string
  css?: string
}

export type BuildTimeRenderingStream =
  | BuildTimeRenderingStreamAttribute
  | BuildTimeRenderingStreamRaw
  | BuildTimeRenderingStreamRepeat
  | BuildTimeRenderingStreamSignal
  | BuildTimeRenderingStreamWhen
  | BuildTimeRenderingStreamComponent

export type BuildTimeRenderingStreamTemplateRecords = Record<string, Array<BuildTimeRenderingStream>>

export interface BuildTimeRenderingProtocol {
  streams: BuildTimeRenderingStream[]
  templates: BuildTimeRenderingStreamTemplateRecords
}
