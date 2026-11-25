;; copy to `dp2m.fnl` to override (falls back to this file if absent)
;; papers: :endura-premier :ektacolor-edge :supra-endura :portra-endura
;;         :crystal-archive :kodak-2383 :kodak-2393 :ultra-endura

{:firmware "data/DP2MV105.bin"
 :out-dir  "out"
 :pack     "data/film_pack.bin"
 :verify   true

 ;; :enabled false hides from menu; :paper sets Auto paper + lv cmcm/curve for negs
 :modes {;; quattro / dp2q (9..13)
         :fov-classic-yellow {:enabled true}
         :cinema             {:enabled true}
         :sunset-red         {:enabled true}
         :forest-green       {:enabled true}
         :teal-and-orange    {:enabled true}

         ;; optical filter mono (14..18)
         :true-mono          {:enabled true}
         :mono-yellow-8      {:enabled true}
         :mono-orange-21     {:enabled true}
         :mono-red-25        {:enabled false}
         :mono-green-11      {:enabled false}

         ;; slide (19..22)
         :velvia-100         {:enabled true}
         :provia-100f        {:enabled true}
         :ektachrome-100     {:enabled true}
         :kodachrome-64      {:enabled true}

         ;; c-41 / ecn-2 neg (23..38)
         :portra-160         {:enabled false :paper :portra-endura}
         :portra-400         {:enabled true :paper :portra-endura}
         :portra-800         {:enabled true :paper :portra-endura}
         :portra-800-p1      {:enabled false :paper :portra-endura}
         :portra-800-p2      {:enabled true :paper :portra-endura}
         :gold-200           {:enabled true :paper :ektacolor-edge}
         :ultramax-400       {:enabled false :paper :ektacolor-edge}
         :ektar-100          {:enabled true :paper :supra-endura}
         :verita-200d        {:enabled true :paper :endura-premier}
         :vision3-50d        {:enabled false :paper :kodak-2383}
         :vision3-250d       {:enabled true :paper :kodak-2383}
         :vision3-200t       {:enabled true :paper :kodak-2383}
         :vision3-500t       {:enabled false :paper :kodak-2383}
         :pro-400h           {:enabled true :paper :crystal-archive}
         :superia-xtra       {:enabled false :paper :crystal-archive}
         :fujicolor-c200     {:enabled false :paper :crystal-archive}

         ;; silver-halide bw (39..43)
         :kodak-trix-400     {:enabled false}
         :ilford-hp5-plus    {:enabled true}
         :fuji-acros-100     {:enabled true}
         :kodak-tmax-100     {:enabled false}
         :rollei-retro-80s   {:enabled false}

         ;; sd lut slots D:\LUT1.BIN..LUT8.BIN (44)
         :sd-custom-lut      {:enabled true}}

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
       ;; apex q16 min shutter: Auto(1/50.8), 1/16, 1/32, 1/64, 1/128, 1/256, 1/512
       :min-shutter-q16   [0x0005AAAA
                           0x00040000
                           0x00050000
                           0x00060000
                           0x00070000
                           0x00080000
                           0x00090000]
       :grid-modes {:level         true
                    :xpan-21x9     true
                    :cinema-16x9   true
                    :square-1x1    true
                    :peaking       true
                    :peaking-level true
                    :zebras        true
                    :rgb-hist      true
                    :frame-7x6     true
                    :frame-4x3     true}}

 :sd {:mount     "/Volumes/SIGMA DP2M"
      :copy-firm true
      :copy-c78  true}}
