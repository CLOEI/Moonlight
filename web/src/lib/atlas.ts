export type AtlasMeta = {
  cell: number
  cols: number
  rows: number
  tiles: Record<string, [number, number]>
}

export type AtlasBundle = {
  meta: AtlasMeta
  image: HTMLImageElement
}

let atlasPromise: Promise<AtlasBundle | null> | null = null
let loaded: AtlasBundle | null = null

function loadOnce(): Promise<AtlasBundle | null> {
  if (atlasPromise) return atlasPromise
  const p = (async (): Promise<AtlasBundle | null> => {
    try {
      const res = await fetch("/tiles/atlas.json", { cache: "force-cache" })
      if (!res.ok) return null
      const meta = (await res.json()) as AtlasMeta
      const image = await new Promise<HTMLImageElement>((resolve, reject) => {
        const img = new Image()
        img.onload = () => resolve(img)
        img.onerror = reject
        img.src = "/tiles/atlas.png"
      })
      return { meta, image }
    } catch {
      return null
    }
  })()
  atlasPromise = p
  p.then((bundle) => {
    loaded = bundle
    if (!bundle) atlasPromise = null
  })
  return p
}

if (typeof window !== "undefined") {
  loadOnce()
}

export function getAtlas(): Promise<AtlasBundle | null> {
  return loadOnce()
}

export function peekAtlas(): AtlasBundle | null {
  return loaded
}
