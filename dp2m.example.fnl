;; copy to `dp2m.fnl` to override (falls back to this file if absent)
;; evaluated by embedded LuaJIT + Fennel; the `dp2m` global exposes the catalog:
;;   dp2m.films  [{:slug :label :kind}]  kinds: quattro mono reversal negative bw sd-lut
;;   dp2m.papers {:negative [...] :bw [...] :sd-lut [...]}  each [{:slug :label}]
;;   dp2m.grids  [{:slug :label}]

(local hidden {:mono-red-25 true :mono-green-11 true
               :portra-160 true :portra-800-p1 true :ultramax-400 true
               :vision3-50d true :vision3-500t true :superia-xtra true :fujicolor-c200 true
               :kodak-trix-400 true :kodak-tmax-100 true :rollei-retro-80s true})

;; "Auto" print paper per film (negatives: color paper, bw: darkroom paper, sd-lut: slot)
(local papers {:portra-400 :portra-endura :portra-800 :portra-endura :portra-800-p2 :portra-endura
               :gold-200 :ektacolor-edge :ektar-100 :supra-endura :verita-200d :endura-premier
               :vision3-250d :kodak-2383 :vision3-200t :kodak-2383 :pro-400h :crystal-archive
               :ilford-hp5-plus :neutral-fiber :fuji-acros-100 :selenium-tone})

{:firmware "data/DP2MV105.bin"
 :out-dir  "out"
 :pack     "data/film_pack.bin"
 :verify   true
 :version  "2.00"

 :films (collect [_ f (ipairs dp2m.films)]
          (values f.slug {:enabled (not (. hidden f.slug)) :paper (. papers f.slug)}))

 :m5 {:grain-scale 1.0
      :halo-scale  1.0
      :sd-lut-defaults {:grain 24
                        :grain-profile :negative ;; :negative | :slide | :bw
                        :halo 64
                        :halo-tint :crimson}}    ;; :crimson | :neutral | :amber

 :m6m {:iso50 true
       :bkt5  true
       :peaking-threshold 44  ;; sobel thresh (lower = more sensitive)
       :zebra-threshold   242 ;; ~95% ire
       ;; apex q16 min shutter: Auto(1/50.8), 1/15, 1/30, 1/60, 1/125, 1/250, 1/500
       :min-shutter-q16 [0x0005AAAA 0x00040000 0x00050000 0x00060000 0x00070000 0x00080000 0x00090000]
       :grids (collect [_ g (ipairs dp2m.grids)] (values g.slug true))}

 :sd {:mount   "/Volumes/SIGMA DP2M"
      :package true    ;; DP2MVxxx.bin, stock updater (shows version)
      :firm    false   ;; loose Firm.bin / C78FRM.bin, debug updater
      :c78     false}}
