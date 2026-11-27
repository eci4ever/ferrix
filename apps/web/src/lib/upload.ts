import { toast } from 'sonner'
import { ApiRequestError } from '@/lib/api'

// XMLHttpRequest exposes actual upload bytes; fetch does not expose upload progress.
export function upload<T>(path: string, file: Blob, onProgress: (percent: number | null) => void, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    const abort = () => xhr.abort()
    const fail = (error: Error) => { toast.error(error.message, { id: `request-error:${path}`, duration: 8000 }); reject(error) }
    xhr.open('POST', `/api/v1${path}`)
    xhr.setRequestHeader('Content-Type', 'application/octet-stream')
    xhr.timeout = 120_000
    xhr.upload.onprogress = event => onProgress(event.lengthComputable ? Math.min(100, Math.floor(event.loaded / event.total * 100)) : null)
    xhr.onload = () => {
      let body: unknown
      try { body = xhr.responseText.trim() ? JSON.parse(xhr.responseText) : undefined } catch { fail(new Error('Ferrix returned an invalid upload response.')); return }
      if (xhr.status < 200 || xhr.status >= 300) {
        const message = body && typeof body === 'object' && 'error' in body && typeof body.error === 'string' ? body.error : `Upload failed (${xhr.status}).`
        fail(new ApiRequestError(message, xhr.status)); return
      }
      resolve(body as T)
    }
    xhr.onerror = () => fail(new Error('Cannot reach Ferrix. Check your connection and retry the upload.'))
    xhr.ontimeout = () => fail(new Error('Upload timed out. Please retry.'))
    xhr.onabort = () => reject(new DOMException('Upload cancelled.', 'AbortError'))
    xhr.onloadend = () => signal.removeEventListener('abort', abort)
    if (signal.aborted) { reject(new DOMException('Upload cancelled.', 'AbortError')); return }
    signal.addEventListener('abort', abort, { once: true })
    xhr.send(file)
  })
}
