# Changelog

## [0.3.0](https://github.com/agusmdev/hydroid/compare/v0.2.0...v0.3.0) (2026-10-15)


### Features

* general catalog concepts replace library-specific rules ([d68e43e](https://github.com/agusmdev/hydroid/commit/d68e43ef501c3900e7923086907b72c3908d8328))
* **hillclimb:** gate fails on Rust literals that name library code ([67def39](https://github.com/agusmdev/hydroid/commit/67def3950ef5c4a016cd234607c0e3ecd162102b))


### Bug fixes

* **analysis:** a function given an exact instance dispatches calls on that parameter by its class ([de65e50](https://github.com/agusmdev/hydroid/commit/de65e5015808b16df5517e7659ed3e52fcc3a71c))
* **analysis:** callables stored in dict/list/tuple literals and called by subscript ([4f5dfbc](https://github.com/agusmdev/hydroid/commit/4f5dfbcce638f1af03bb8c1478d31ca09d03055a))
* **analysis:** classes library code constructs, and classes never instantiated themselves ([ccbd4d1](https://github.com/agusmdev/hydroid/commit/ccbd4d1ca98c164ffca20397f41ab45b0781c6b9))
* **analysis:** cooperative super() chains along the receiver's MRO; overrides limited to the receiver's subclasses ([a774ba4](https://github.com/agusmdev/hydroid/commit/a774ba4548eec7d76ae9b32e5a08ea2494328ecb))
* **analysis:** default factories run by construction/validation, contextvars Context.run ([51d8373](https://github.com/agusmdev/hydroid/commit/51d837350a50cd3120c82872239e9887c633690e))
* **analysis:** dispatch tables with spread or starred elements keep ty's targets ([236170a](https://github.com/agusmdev/hydroid/commit/236170a0a63330d9e001c46c46d9431a4bff4b5f))
* **analysis:** exits registered on an exit stack run where the stack is closed ([62c801d](https://github.com/agusmdev/hydroid/commit/62c801decce19c4085763eb57c2976cffa4276a8))
* **analysis:** lazy iterables consumed by offload APIs, generator __iter__ and [@contextmanager](https://github.com/contextmanager) factories run nothing until consumed ([db87cac](https://github.com/agusmdev/hydroid/commit/db87cacbba3b0336103f1dead28eca3b8a85b512))
* **analysis:** library overrides inherit their base's sink; unannotated __enter__ returning self; tenacity wraps ([48ae1ca](https://github.com/agusmdev/hydroid/commit/48ae1cac3bf1fbbf2a02d0e26b4dc11abb21d124))
* **analysis:** logging calls run attached project handlers, filters and formatters ([2f21444](https://github.com/agusmdev/hydroid/commit/2f21444d7049625b656954bf8cf85db16357bfd5))
* **analysis:** logging hooks scoped to the logger hierarchy; relationships loaded off the loop are not lazy loads ([ea3afe3](https://github.com/agusmdev/hydroid/commit/ea3afe302527c0d59abc950bdb404dbe37897c10))
* **analysis:** nested and Annotated pydantic hooks, explicit dumps; response_model=None, Response returns and response_model_exclude skip response hooks ([ed26647](https://github.com/agusmdev/hydroid/commit/ed2664763b252eb42ff93bc8927666000813b690))
* **analysis:** offloading decorators and running-loop guards are not loop code ([866e567](https://github.com/agusmdev/hydroid/commit/866e5675dcbcd66724161a1e669b8eae8869b39b))
* **analysis:** per-instance method specialisation for exact receivers and specialised generics ([d0ecf74](https://github.com/agusmdev/hydroid/commit/d0ecf74cde2c140d06d83b75321070932a432a8a))
* **analysis:** project callables replacing library methods on instances; lazy iterables streamed by StreamingResponse run off the loop ([387d242](https://github.com/agusmdev/hydroid/commit/387d242d3ff18bfbde97f4740401a2257d56470e))
* **analysis:** project overrides of hooks that library async methods call on self ([fb0e28e](https://github.com/agusmdev/hydroid/commit/fb0e28ea834c6838c9fbcad6797e84133153086e))
* **analysis:** pydantic dumps honour when_used, mode, exclude/include; nested instances are not revalidated ([e6786d5](https://github.com/agusmdev/hydroid/commit/e6786d593afce68652ad64ef237e590b02f26fbe))
* **analysis:** pydantic hooks FastAPI runs on the loop (request validation, response validation and serialization) ([587b060](https://github.com/agusmdev/hydroid/commit/587b060b835bd712fbca5e73f0978cc2728df746))
* **analysis:** response classes FastAPI constructs on the loop (render, init_headers, __init__) ([4e166f2](https://github.com/agusmdev/hydroid/commit/4e166f2c30ced981abc8a8057e482c2cb9d1df12))
* **analysis:** SQLAlchemy lazy relationship loads and [@validates](https://github.com/validates) hooks ([9609da4](https://github.com/agusmdev/hydroid/commit/9609da4ff749a4c36608719330e7c96dc59c921a))
* **analysis:** stdlib helpers that block internally; middleware constructed on the loop ([2a21561](https://github.com/agusmdev/hydroid/commit/2a215618062eb687568240a7176f00e159978822))
* **analysis:** type request.state / app.state / request.app reads from the project's stores ([34f69d1](https://github.com/agusmdev/hydroid/commit/34f69d19c3bd8d117204f0d9a9c21e5284ef6855))
* **analysis:** type unannotated Depends/Security parameters from their provider ([b7074e7](https://github.com/agusmdev/hydroid/commit/b7074e7b4a35e2b865102b9f12b3d1e0ba9c553a))
* **hillclimb:** key the holdout ratchet by the holdout case set ([1884894](https://github.com/agusmdev/hydroid/commit/1884894b3e1a4dd3c5a1fdd167e1c2bb5bddca18))
* **hillclimb:** oracle no longer reports calls that cannot block ([bbbcc19](https://github.com/agusmdev/hydroid/commit/bbbcc1998b7e59ec682ef8c1a33f59c78008d8fe))
* **hillclimb:** oracle records blocking inside awaited library coroutines again ([7311394](https://github.com/agusmdev/hydroid/commit/7311394442cdcb5aa3a2fe030f7da7a1b3daa33f))
* **index:** a &lt;pkg&gt;-stubs package provides the module &lt;pkg&gt; ([7213f28](https://github.com/agusmdev/hydroid/commit/7213f2877f04fc0c0a3c6bd315c0e38423d4d04e))


### Performance

* **analysis:** make per-instance copies in place instead of cloning the facts ([0feab00](https://github.com/agusmdev/hydroid/commit/0feab00b6bf4d16c20bf3faa31b7c134dca94c45))
* **analysis:** memoize class hooks, look up catalog roles once per function ([ae1d429](https://github.com/agusmdev/hydroid/commit/ae1d42919367794e6d5e2395d46fb79f5f60a603))


### Documentation

* --strict skips awaited calls; review follow-ups in the handoff ([8bd8cb6](https://github.com/agusmdev/hydroid/commit/8bd8cb67cfbb8b897a47c774492397e3e531b209))
* catalog-based examples and known limits match the code ([175015a](https://github.com/agusmdev/hydroid/commit/175015a08b6373a45cb5584070a7ccdbcf94857a))
* final hillclimb handoff and release checklist ([01e5fd5](https://github.com/agusmdev/hydroid/commit/01e5fd534167ba55ba754cb7eb07acb7f83c4cd7))
* **hillclimb:** audit of library-specific rules and warm-time growth (phase A) ([3093ac1](https://github.com/agusmdev/hydroid/commit/3093ac15d56e157967879e017ddaaef8567adced))
* **hillclimb:** loop policy for holdout FPs, library rules, triage, Polar and branch ([b1ba25f](https://github.com/agusmdev/hydroid/commit/b1ba25fdda6abaad486ace6a4e8383f87c861ba4))
* install from PyPI ([90e77dd](https://github.com/agusmdev/hydroid/commit/90e77dd2f9acdbaa583cc6670e9720fe3a1f75cc))
* install from PyPI ([ed51f8d](https://github.com/agusmdev/hydroid/commit/ed51f8df01ebf57a74b1eb4b8790c495fdefe79b))
* README says how hydroid works in a few mechanisms, the guide gives the detail ([e7e3132](https://github.com/agusmdev/hydroid/commit/e7e313236288cb8d3e0c79e976f634e7c1336dbf))

## [0.2.0](https://github.com/agusmdev/hydroid/compare/v0.1.0...v0.2.0) (2026-10-12)


### Features

* adversarial suite, recall fixes, fact cache and faster extraction ([f80c0da](https://github.com/agusmdev/hydroid/commit/f80c0da4ce99d1dad78b3f6ccf7042a0b078c0aa))
* descriptors, __next__, contextlib, singledispatch, bound lambdas, IO ([9e3791d](https://github.com/agusmdev/hydroid/commit/9e3791dff4f632de08aca36c8e802d04d46dc9ec))
* operators, __getattr__, returned callables and classes, list flows ([ff4337a](https://github.com/agusmdev/hydroid/commit/ff4337a34716c2bc019443a135c2cc874e77e4f5))


### Bug fixes

* precision and speed from five more real codebases ([2fdf251](https://github.com/agusmdev/hydroid/commit/2fdf251500746ab6eb297c5530e73abc3be7f15e))


### Documentation

* agent setup prompt and CI/CD reference ([88243fc](https://github.com/agusmdev/hydroid/commit/88243fc82e35b41220c62015a50ae18ccd31c6e6))
* CI/CD integration guide and coding-agent setup prompt ([5a33a4a](https://github.com/agusmdev/hydroid/commit/5a33a4a13707eb53644ad258efa42623ea62cf8d))
* install from prebuilt release wheels ([d47c904](https://github.com/agusmdev/hydroid/commit/d47c9049d27e2c959e34f445decd18331421c323))
* **readme:** CI/CD and agent setup section ([b87002c](https://github.com/agusmdev/hydroid/commit/b87002cfc2b1e604b7cc8c0c84a633301381d7b8))

## 0.1.0 (2026-09-29)

First release: prebuilt wheels on the [GitHub release](https://github.com/agusmdev/hydroid/releases/tag/v0.1.0).
