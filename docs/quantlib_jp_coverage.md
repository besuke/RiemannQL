# 日本語 QuantLib 解説サイト更新履歴（2019年3月〜2026年2月）と rmql の照合（2026-11-21）

日本語の金融工学・QuantLib 解説サイト（「基礎編」「上級編」「実践編：QuantLibを使ってみる」「Implementing QuantLib 和訳」）の 2019年3月〜2026年2月の更新履歴（全 46 項目）と、`rmql` の現在の実装（`crates/rmql` および R 公開 API）を突き合わせた記録である。

- **区分**：本書の照合記述はすべて **【ソース確認】**（2026-11-21 に Rust 中核・R 公開関数・サンプルスクリプトのソースを読んで確認）。
- **未実施の範囲**：更新履歴からリンクされる各章本文の確認、R での再実行、および本記録作成時の `cargo test` 再実行は行っていない。
- **関連文書**：英語版 QuantLib Guide と公式 Examples の棚卸しは `docs/quantlib_guide_inventory.md`、Coral Book との照合範囲は `docs/lagrangefinance_coverage.md` にある。

---

## 1. 2019年（#1〜#11）

| # | 更新 | 判定 | rmql との照合 | 参照 |
|---|---|---|---|---|
| 1 | 2019年3月・イールドカーブ補間 | 一部対応 | ゼロ金利とスプレッドの線形補間、割引係数の対数線形・単調対数 3 次補間は実装済み。Tension Spline、Monotone Convex、Minimalist Quadratic は未実装（単調対数 3 次と Monotone Convex は別方式）。 | `R/curves.R:28`、`crates/rmql/src/math/interpolation.rs` |
| 2 | 2019年4月・導入と `EquityOption` | 一部再現 | Black–Scholes、Barone–Adesi–Whaley、Bjerksund–Stensland（1993年版）、CRR 二項木の比較用サンプルがある。Heston、Bates、有限差分などは未実装。 | `crates/rmql/src/american_approximations.rs:1`、`inst/examples/quantlib_examples/equity_option.R:1` |
| 3 | 2019年4月・Implementing QuantLib I–II | 設計比較 | 商品条件・市場データ・評価関数に分けて同じ計算を組み立てられる。`Instrument` と `PricingEngine` の継承・差し替え機構は設計上の対象外。 | `docs/specification.md`（原則 R1〜R5） |
| 4 | 2019年5月・Turn of Year／III Term Structures | カーブ基盤は対応（Turn of Year は未対応） | フラット・ゼロ金利・割引係数・スプレッド付きカーブを構築できる。期末日を指定した割引係数のジャンプ（Turn of Year）は未実装。 | `crates/rmql/src/curves.rs:26` |
| 5 | 2019年6月・Multi-Curve／`MulticurveBootstrapping`／IV Cash Flows | 一部対応 | OIS の予測・割引カーブ分離、FX スワップおよび定額・MtM 元本の通貨ベーシスからのカーブ構築、キャッシュフロー表と NPV は実装済み。原例の Euribor 予測カーブは対象外。同一通貨の OIS ベーシスによる汎用ブートストラップも対象外。 | `R/ois_swaps.R:133`、`R/xccy.R:27` |
| 6 | 2019年7月・V 校正／VI モンテカルロ | 計算機能は対応 | Hull–White、SABR、LMM、PRDC にモデル別の校正がある。乱数、経路生成、モンテカルロ評価も実装済み。QuantLib の校正・MC クラス階層の再現は設計上の対象外。 | `crates/rmql/src/hull_white.rs`、`crates/rmql/src/sabr.rs`、`crates/rmql/src/lmm_market.rs`、`crates/rmql/src/prdc.rs` |
| 7 | 2019年8月・無裁定価格理論、FTAP | 理論の適用例あり | LMM の Spot・Terminal 測度で、ニューメレールに応じてドリフトと割引を切り替える。 | `crates/rmql/src/lmm_market.rs:79` |
| 8 | 2019年9月・VII Tree Framework | 一部対応 | 株式オプションの CRR 二項木はヨーロピアン型とアメリカン型に対応。汎用 3 項木は未実装。Hull–White の早期行使には状態格子を使う。 | `crates/rmql/src/options.rs:489` |
| 9 | 2019年9月・`Bonds` | 一部再現 | 固定利付債、ステップアップ、アモチ、キャッシュフロー、利回り、カーブ評価は対応。原例の IBOR 変動利付債は対象外。 | `crates/rmql/src/instruments/fixed_rate_bond.rs`、`crates/rmql/src/bond_curve.rs` |
| 10 | 2019年10月・VIII 有限差分／IX 結論 | 有限差分は未実装 | PDE ソルバー（有限差分法）は未実装。二項木、状態格子上の求積、MC は利用できる。IX は総括のため設計資料扱い。 | — |
| 11 | 2019年12月・Appendix A／B | 設計比較 | デザインパターンやコード規約の補遺。実装件数の集計から外し、設計資料として扱う。 | `docs/specification.md` |

---

## 2. 2020〜2021年（#12〜#23）

| # | 更新 | 判定 | rmql との照合 | 参照 |
|---|---|---|---|---|
| 12 | 2020年2月・Cap／Swaption の Black モデル | 主要な価格計算に対応 | Black・Bachelier のキャップレット、ヨーロピアン・スワップションの評価がある。後決め RFR キャップレットも実装済み。専用の Cap 一括評価入口はなく、各キャップレットを集計する。 | `crates/rmql/src/options.rs`、`crates/rmql/src/lmm.rs`、`crates/rmql/src/fmm.rs` |
| 13 | 2020年10月・Short Rate Models／Vasicek | 一部対応 | ガウス型短期金利モデルは 1 因子 Hull–White で扱える。Vasicek 固有のパラメータを受け取る独立モデルは未実装（信用ポートフォリオの Vasicek モデルとは別項目）。 | `crates/rmql/src/hull_white.rs:45` |
| 14 | 2020年10月・Hull–White 解析解 | 対応 | Rust に割引債価格、R に割引債オプションと Jamshidian のヨーロピアン・スワップションがある。スワップションは $a$・$\sigma$ の自動微分も返す（時間依存ドリフトによる初期カーブ適合と、測度の下での無裁定価格付けは区別する）。 | `crates/rmql/src/hull_white.rs:45`、`R/rate_models.R:64` |
| 15 | 2020年11月・3 項木とバーミューダン | 商品に対応、方式が異なる | 状態格子上のバックワード・インダクションを使う。R の既定はバーミューダンが台形則（`spacing = 0.05`）、コール債が Gauss–Hermite（`nodes = 24, spacing = 0`）。3 項木は未実装。 | `R/rate_models.R:21`、`R/rate_models.R:78` |
| 16 | 2020年12月・時間依存 $a(t), \sigma(t)$ の校正 | 一部対応 | 定数 $a, \sigma$ をヨーロピアン・オプション価格に Levenberg–Marquardt と自動微分で校正する（$a$ 固定で $\sigma$ のみ校正も可）。3 項木による区分定数 $a(t), \sigma(t)$ の校正は未実装。 | `R/rate_models.R:110` |
| 17 | 2021年1月・期待値演算、Feynman–Kac／フーリエ | 理論編（汎用エンジン未実装） | モデル別の解析式や数値求積はある。汎用の PDE ソルバーと特性関数からのフーリエ価格評価エンジンは未実装。 | — |
| 18 | 2021年1月・`CallableBonds` | 一部再現 | コール付き・コールなしの価格、発行体コールの価値、行使判断と境界まで取得可能。市場価格からコール債 OAS を逆算する専用 API は未実装。 | `R/rate_models.R:87`、`R/rate_models.R:95` |
| 19 | 2021年2月・`BermudanSwaption` | 一部再現 | ヨーロピアン・スワップションで 1 因子 Hull–White を校正し、バーミューダンを評価できる。G2++、Black–Karasinski、3 項木・有限差分を含む原例の比較は未再現。 | `R/rate_models.R:78` |
| 20 | 2021年3月・Affine Term Structure Model | 一部対応 | Hull–White の割引債価格は短期金利に対して指数アフィン形で実装済み。CIR と章で扱う拡張モデルは未実装（※アフィン性だけで短期金利が非負になるわけではない点に留意）。 | `crates/rmql/src/hull_white.rs:86` |
| 21 | 2021年4月・対数正規型短期金利モデル | 未実装 | BDT、Black–Karasinski ともに専用モデル・校正なし。 | — |
| 22 | 2021年6月・Markov Functional | 未実装 | オプション市場価格から状態変数と金利・債券価格の写像を構築して評価する機能はない。 | — |
| 23 | 2021年7月・`Gaussian1dModels` | 例は未再現 | 通常の 1 因子 Hull–White バーミューダン評価はある。一般化 GSR、Markov Functional、CMS を使った原例は未対応。 | — |

---

## 3. 2022〜2023年（#24〜#35）

| # | 更新 | 判定 | rmql との照合 | 参照 |
|---|---|---|---|---|
| 24 | 2022年1月・LMM 導入と測度変換 | モデル部分は対応 | 期間フォワードを入力に Spot・Terminal 測度を選べる。後決め RFR の FMM もある。廃止済み LIBOR のカーブ・fixing 管理は対象外。 | `crates/rmql/src/lmm_market.rs:46` |
| 25 | 2022年2月・共通測度のドリフト | 対応 | Rust が Spot・Terminal 両測度の連立ドリフトを計算し、R からの経路生成にも反映する。保存済み参照値との照合テストあり。 | `crates/rmql/src/lmm_market.rs:457` |
| 26 | 2022年4月・古典的 LMM | 対応 | 相関つき対数正規型と正規型、一定ボラと区分定数のボラ格子を選べる。スマイルを動学に組み込む確率ボラ LMM は未実装。 | `crates/rmql/src/lmm_market.rs:68` |
| 27 | 2022年5月・相関の平滑化とランク縮約 | 一部対応 | 指数相関と固有値によるランク縮約は実装済み。過去のカーブ系列からの相関推定や、スプレッドオプション価格からの逆算を行う専用入口はない。 | `crates/rmql/src/lmm_market.rs` |
| 28 | 2022年6月・LMM 校正 | 一部対応 | 相関の減衰係数をスワップションボラに合わせる機能と、相関を固定した Brigo–Mercurio の逐次（cascade）校正がある。後者は 1 期間物を含む完全なクオート三角形を要求する。更新記事の Andersen–Piterbarg 方式とは校正手順が異なる。 | `crates/rmql/src/lmm_market.rs:975` |
| 29 | 2022年10月・LMM モンテカルロ高速化 | 一部対応 | LMM 経路で因子数を縮約できる。経路生成は擬似乱数で逐次処理する。Sobol と Brownian Bridge は株式の Black–Scholes 経路で使える。 | `crates/rmql/src/lmm_market.rs`、`crates/rmql/src/math/sobol.rs` |
| 30 | 2022年12月・離散化バイアス | 対策あり | LMM/FMM で Predictor–Corrector を選べる。R API の既定値は `"euler"` なので、対策を使うには明示指定が必要。 | `R/lmm_market.R:94` |
| 31 | 2023年1月・Longstaff–Schwartz | 対応 | 株式のアメリカン型・バーミューダン、LMM/FMM のバーミューダン、3 因子 PRDC の発行体コールを R から評価できる。株式では回帰用と評価用の経路を分ける設定もある。 | `R/longstaff_schwartz.R:45`、`crates/rmql/src/lmm_market.rs`、`crates/rmql/src/prdc.rs` |
| 32 | 2023年4月・MC 感応度 | 一部対応 | ヨーロピアン型 1 ステップ MC と基本 PRDC に経路別 AAD がある。3 因子コーラブル PRDC は、スポット・カーブに経路別微分（＋カーブ AD）、ボラ・金利パラメータ・相関に共通乱数（CRN）による中心差分を使う。ヨーロピアン型 MC のガンマにはペイオフの平滑化（`smoothing`）が必要。Likelihood Ratio 法は未実装。 | `crates/rmql/src/options.rs:608`、`crates/rmql/src/prdc.rs:1018` |
| 33 | 2023年10月・Dupire／Heston | 関連機能あり（本体未実装） | SABR のインプライド・ボラ計算と校正、FX スマイル補間は実装済み。Dupire の局所ボラ、Heston の価格評価・経路生成は未実装。 | `crates/rmql/src/sabr.rs`、`crates/rmql/src/fx_smile.rs` |
| 34 | 2023年11月・導入手順と `EquityOption` 改訂 | 一部再現 | 現行の R サンプルで解析解、アメリカン型近似 2 方式（BAW・Bjerksund–Stensland 1993）、CRR を比較できる。原例に列挙された全エンジンの比較は未再現。 | `inst/examples/quantlib_examples/equity_option.R:1` |
| 35 | 2023年12月・`MulticurveBootstrapping` 改訂 | OIS 部分を再現可能 | 予測・割引分離とスワップ評価を R で組める。改訂版の Euribor 側のヘルパー・予測カーブは対象外（#5 を参照）。 | `R/ois_swaps.R:133` |

---

## 4. 2024〜2026年（#36〜#46）

| # | 更新 | 判定 | rmql との照合 | 参照 |
|---|---|---|---|---|
| 36 | 2024年1月・`Bonds` 改訂 | 一部再現 | 固定利付債の価格・経過利息・日程・キャッシュフローは対応。割引債の専用コンストラクターはなく、IBOR 変動利付債は対象外。 | `crates/rmql/src/instruments/fixed_rate_bond.rs` |
| 37 | 2024年4月・`CallableBonds`／`BermudanSwaption` 改訂 | 一部再現 | 外生パラメータでのコール債評価と、ヨーロピアン・スワップション校正後の 1 因子 Hull–White バーミューダン評価が可能。3 モデル比較と原例の数値エンジン（3 項木・有限差分）は未再現。 | `R/rate_models.R:78`、`R/rate_models.R:87` |
| 38 | 2024年8月・`Gaussian1dModels` 改訂 | 例は未再現 | 通常の 1 因子 Hull–White 評価はある。`calibrationBasket()` の自動生成、GSR／Markov Functional、CMS 評価は未実装。 | — |
| 39 | 2024年9月・`Replication` | 部品のみ対応 | ヨーロピアン・プットとバリアプットの価格は計算できる。デジタルオプションの専用価格 API と静的複製ポートフォリオの構築は未実装。 | `crates/rmql/src/options.rs` |
| 40 | 2024年10月・`FittedBondCurve` | 未実装 | 債券価格からのブートストラップはある。Nelson–Siegel、Svensson、スプラインなどのパラメトリック当てはめは未実装。 | `crates/rmql/src/bond_curve.rs` |
| 41 | 2024年10月・`DiscreteHedging` | 例は未再現 | 株価経路とデルタは取得できる。リバランスごとの株式・現金残高を追い、満期の複製誤差分布を返す専用機能は未実装。 | — |
| 42 | 2024年12月・実践編 `CDS` | 価格計算に対応 | MidPoint／ISDA、スプレッド・アップフロントからのハザードカーブ構築がある。 | `crates/rmql/src/cds.rs`、`R/cds.R:36` |
| 43 | 2025年3月・基礎編 CDS | 基礎編の計算に対応 | CDS の満期・日程、プレミアムとプロテクションの各レッグ、生存確率、NPV を扱える。 | `R/cds.R:36` |
| 44 | 2025年10月・CDO トランシェ入門 | 対応 | アタッチメント・デタッチメントによる損失配分、期待損失、VaR・ES を計算できる。 | `crates/rmql/src/tranche.rs` |
| 45 | 2025年12月・Merton／Vasicek／正規コピュラ | 一部対応 | 1 因子正規モデルの条件付き PD と有限ポートフォリオ分布、ASRF の VaR、大規模均質ポートフォリオ（LHP）のトランシェ期待損失を実装済み。企業価値から個社 PD を出す Merton 構造モデルは未実装。 | `crates/rmql/src/credit_risk.rs`、`crates/rmql/src/tranche.rs:90` |
| 46 | 2026年2月・Gaussian Latent Variable Model | 主要部対応 | 個別 PD・EAD・LGD・因子感応度から条件付き損失分布を再帰計算し、共通因子を複合 Gauss–Legendre 求積で積分する（各銘柄の $\mathrm{EAD} \times \mathrm{LGD}$ は指定する損失単位の整数倍が必要）。トランシェの両レッグとフェアスプレッドを計算できるが、入力は銘柄別の定数ハザード率とフラット割引金利。確率的回収率や市場カーブとの直接連携は未実装。 | `crates/rmql/src/credit_risk.rs:134`、`crates/rmql/src/tranche.rs:148` |

---

## 5. 主な未実装領域の整理

1. **カーブ高度化**：Turn of Year ジャンプ、Monotone Convex / Tension Spline、`FittedBondCurve`（Nelson–Siegel / Svensson / B-Spline 等）。
2. **数値解法エンジン**：有限差分法（FDM / PDE）、汎用 3 項木フレームワーク、特性関数フーリエ変換エンジン、MC の Likelihood Ratio 法（LRM）。
3. **ボラティリティ・金利モデル拡張**：Dupire Local Volatility、Heston、G2++、Black–Karasinski、BDT、CIR、Markov Functional（`Gaussian1dModels`）、区分定数 $a(t), \sigma(t)$ の Hull–White 校正、コール債 OAS 逆算。
4. **クレジット拡張**：Merton 構造モデル、確率的回収率、多因子／t コピュラ、Base Correlation 校正、トランシェ評価と市場カーブの直接連携。
