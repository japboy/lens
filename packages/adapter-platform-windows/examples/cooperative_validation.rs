//! Interactive Windows validation of the cooperative adapter through the shared usecase.
//! The harness owns admission and exports accepted contexts to local evidence files.

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("The cooperative fixture requires Windows x64.");
    std::process::exit(1);
}

#[cfg(all(windows, target_arch = "x86_64"))]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    validation::run().await
}

#[cfg(all(windows, target_arch = "x86_64"))]
mod validation {
    use adapter_platform_windows::{
        marker_oracle::{self, Phase, Rect, Rgba},
        Fixture, FixturePlatform, FixtureReadProbe,
    };
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use domain::{
        lens::{LensInput, LensMediaGeometry, LensTargetSet, TargetNamespace},
        projection::LensAgentProjection,
    };
    use port_platform::{
        authority::{ReadSequence, TargetReadKey},
        observation::{
            Observation, ObservationRequest, ObservationSession, WindowObservationNotification,
        },
        selection::{TargetSelection, WindowPickerReply},
        trust::AccessibilityTrust,
    };
    use serde_json::json;
    use std::{
        collections::BTreeMap,
        env, fs,
        future::{poll_fn, Future},
        num::NonZeroU64,
        path::Path,
        pin::Pin,
        sync::Arc,
        time::Duration,
    };
    use usecase::{
        context::{
            build_context, BlockingExecutor, BuiltContext, ContextBuildRequest, WindowAccessMode,
        },
        platform::picker_reply,
        read_admission::{
            CleanupOutcome, OperationAdmission, OperationTerminalReason, ReadAdmission,
        },
    };
    use uuid::Uuid;

    struct Worker;

    impl BlockingExecutor for Worker {
        fn run<T: Send + 'static>(
            &self,
            _: Option<TargetReadKey>,
            work: impl FnOnce() -> T + Send + 'static,
        ) -> Pin<Box<dyn Future<Output = Result<T, String>> + Send>> {
            Box::pin(async move {
                tokio::task::spawn_blocking(work)
                    .await
                    .map_err(|error| error.to_string())
            })
        }
    }

    struct Session {
        platform: Arc<FixturePlatform>,
        admission: ReadAdmission,
        observation: ObservationSession,
        request: ObservationRequest,
        target_set: LensTargetSet,
        fixture: Fixture,
        published: Option<TargetReadKey>,
    }

    impl Session {
        fn start_read(&mut self, revision: u64) -> Result<TargetReadKey, String> {
            let read = self
                .admission
                .issue(&self.target_set.targets[0].id, revision)?;
            check(self.admission.start(read), "common read did not start")?;
            log("read-started", json!({"read":read,"revision":revision}));
            Ok(read)
        }

        fn fence(&mut self, reason: OperationTerminalReason) {
            if matches!(
                self.admission.operation_state(),
                OperationAdmission::Terminal(_)
            ) {
                return;
            }
            self.admission.terminate(reason);
            let withdrawn = self.published.take();
            log(
                "publication-closed",
                json!({"target":self.request.target,"reason":format!("{reason:?}"),"withdrawn":withdrawn}),
            );
        }
    }

    struct Candidate {
        built: BuiltContext,
        projection: LensAgentProjection,
        read: TargetReadKey,
        raw_png: Vec<u8>,
    }

    fn check(condition: bool, message: &str) -> Result<(), String> {
        if condition {
            Ok(())
        } else {
            Err(message.into())
        }
    }

    fn log(event: &str, detail: serde_json::Value) {
        println!("{}", json!({"event": event, "detail": detail}));
    }

    async fn select(
        platform: FixturePlatform,
        fixture: Fixture,
    ) -> Result<Session, Box<dyn std::error::Error>> {
        let platform = Arc::new(platform);
        check(!platform.inspect(), "unselected fixture reported ready")?;
        let operation = Uuid::new_v4();
        let mut admission = ReadAdmission::new(operation);
        let mut registered = false;
        let selected: Result<_, Box<dyn std::error::Error>> = async {
        let reply = platform.pick(operation).await?;
        registered = matches!(&reply, WindowPickerReply::Selected { .. });
        let WindowPickerReply::Selected { windows } = &reply else {
            return Err("positive fixture selection must be explicitly confirmed".into());
        };
        check(
            windows.len() == 1 && platform.inspect(),
            "one live fixture required",
        )?;
        let target = windows[0].target;
        let target_set = LensTargetSet::try_new_for_namespace(
            operation,
            picker_reply(reply).into_selected()?.ok_or("no fixture")?,
            TargetNamespace::Windows,
        )?;
        admission.register(target_set.targets[0].id.clone(), target)?;
        let request = ObservationRequest {
            target,
            context_id: operation,
            source_registration_id: Uuid::new_v4(),
            observer_epoch: NonZeroU64::MIN,
        };
        let observation = platform.observe(request.clone())?;
        log(
            "registered",
            json!({"target":target,"fixture":format!("{fixture:?}"),"source_registration_id":request.source_registration_id,"observer_epoch":request.observer_epoch}),
        );
        Ok((observation, request, target_set))
        }.await;
        match selected {
            Ok((observation, request, target_set)) => Ok(Session {
                platform,
                admission,
                observation,
                request,
                target_set,
                fixture,
                published: None,
            }),
            Err(error) => {
                admission.terminate(OperationTerminalReason::AuthorityUncertain);
                admission.begin_cleanup();
                let released = platform.release_operation(operation);
                admission.finish_cleanup(if registered && released.is_ok() {
                    CleanupOutcome::ConfirmedClosed
                } else {
                    CleanupOutcome::UnconfirmedTermination
                });
                log(
                    "cleanup",
                    json!({"operation":operation,"phase":"selection","outcome":format!("{:?}",admission.cleanup_state()),"scenario_error":error.to_string(),"registered_release":registered,"unregistered_teardown_unobserved":!registered,"release_error":released.as_ref().err().map(ToString::to_string)}),
                );
                match released {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(format!("{error}; cleanup failed: {cleanup}").into()),
                }
            }
        }
    }

    async fn acquire(
        platform: Arc<FixturePlatform>,
        target_set: &LensTargetSet,
        fixture: Fixture,
        read: TargetReadKey,
        evidence: &Path,
    ) -> Result<Candidate, Box<dyn std::error::Error>> {
        let id = target_set.targets[0].id.clone();
        let revision = read.sequence.get();
        let built = build_context(
            &Worker,
            platform.clone(),
            platform.clone(),
            ContextBuildRequest {
                operation_id: read.target.operation_id,
                context_revision: revision,
                source_revisions: BTreeMap::from([(id.clone(), revision)]),
                target_set,
                access_mode: WindowAccessMode::Registered {
                    reads: BTreeMap::from([(id, read)]),
                },
            },
        )
        .await?;
        let context = serde_json::to_value(&built.context)?;
        check(built.reads == [read], "common context lost issued read")?;
        let updated = revision.is_multiple_of(2);
        let expected_marker = if updated {
            "\"Lens geometry marker updated\""
        } else {
            "\"Lens geometry marker\""
        };
        check(
            context.to_string().contains(expected_marker),
            "UIA text did not match the fixture revision",
        )?;
        check(
            built.payloads.len() == 1 && built.context.media.len() == 1,
            "paired WGC PNG was not assembled",
        )?;
        let input = LensInput::from_context(&built.context).ok_or("common input missing")?;
        let projection =
            LensAgentProjection::from_input(&input, &built.target_set, &built.payloads)?;
        check(
            projection.prompt_media().len() == 1,
            "projection dropped the paired PNG",
        )?;
        let png = STANDARD.decode(&built.payloads[0].data)?;
        let rgba = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?.into_rgba8();
        let phase = match (fixture, updated) {
            (_, true) => Phase::Updated,
            (Fixture::A, false) => Phase::A,
            (Fixture::B, false) => Phase::B,
        };
        let raw_png = platform.validation_frame_png(read)?;
        let raw =
            image::load_from_memory_with_format(&raw_png, image::ImageFormat::Png)?.into_rgba8();
        let LensMediaGeometry::Acquired { crop, .. } = &built.context.media[0].geometry else {
            return Err("fixture media has no acquired crop geometry".into());
        };
        let reported = Rect {
            x: crop.x,
            y: crop.y,
            width: crop.width,
            height: crop.height,
        };
        let detected = marker_oracle::verify(
            Rgba::new(raw.width(), raw.height(), raw.as_raw())?,
            Rgba::new(rgba.width(), rgba.height(), rgba.as_raw())?,
            reported,
            phase,
        );
        let detected = match detected {
            Ok(rect) => rect,
            Err(error) => {
                // Diagnostic artifacts are explicitly not accepted publications.
                // Preserve them before teardown removes the private native scratch.
                let name = format!(
                    "oracle-rejected-{}-{}",
                    read.target.operation_id,
                    read.sequence.get()
                );
                fs::write(evidence.join(format!("{name}-frame.png")), &raw_png)?;
                fs::write(evidence.join(format!("{name}-capture.png")), &png)?;
                fs::write(
                    evidence.join(format!("{name}-context.json")),
                    serde_json::to_vec_pretty(&built.context)?,
                )?;
                fs::write(
                    evidence.join(format!("{name}-error.json")),
                    serde_json::to_vec_pretty(
                        &json!({"status":"oracle_rejected_not_published","read":read,"detail":error}),
                    )?,
                )?;
                log(
                    "pixel-oracle-rejected",
                    json!({"read":read,"detail":error,"diagnostic_prefix":name}),
                );
                return Err(error.into());
            }
        };
        log(
            "common-context-built",
            json!({"read":read,"revision":built.context.revision,"digest":projection.digest().as_str(),"fixture":format!("{fixture:?}"),"extent":[rgba.width(),rgba.height()],"raw_extent":[raw.width(),raw.height()],"pixel_oracle_crop":[detected.x,detected.y,detected.width,detected.height],"asymmetric_marker_verified":true}),
        );
        Ok(Candidate {
            built,
            projection,
            read,
            raw_png,
        })
    }

    fn accept(session: &mut Session, candidate: &Candidate) -> Result<(), String> {
        let key = candidate.read;
        if !session.admission.allows_all(&candidate.built.reads) {
            return Err("stale, terminal or duplicate common read".into());
        }
        session
            .platform
            .approve_read_publication(key)
            .map_err(|error| error.to_string())?;
        check(
            session.admission.accept_all(&candidate.built.reads),
            "common commit rejected validated read",
        )?;
        session.published = Some(key);
        Ok(())
    }

    fn export(
        pending: &Candidate,
        evidence: &Path,
        name: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        fs::write(
            evidence.join(format!("{name}-context.json")),
            serde_json::to_vec_pretty(&pending.built.context)?,
        )?;
        fs::write(
            evidence.join(format!("{name}-capture.png")),
            STANDARD.decode(&pending.built.payloads[0].data)?,
        )?;
        fs::write(evidence.join(format!("{name}-frame.png")), &pending.raw_png)?;
        log(
            "published",
            json!({"read":pending.read,"case":name,"digest":pending.projection.digest().as_str()}),
        );
        Ok(())
    }

    fn close(session: &mut Session, scenario_error: Option<&str>) -> Result<(), String> {
        session.admission.begin_cleanup();
        let operation = session.admission.operation_id();
        let observed = session.observation.registration.close();
        let released = session.platform.release_operation(operation);
        session
            .admission
            .finish_cleanup(if observed.is_ok() && released.is_ok() {
                CleanupOutcome::ConfirmedClosed
            } else {
                CleanupOutcome::UnconfirmedTermination
            });
        log(
            "cleanup",
            json!({"operation":operation,"outcome":format!("{:?}", session.admission.cleanup_state()),"scenario_error":scenario_error,"observer_error":observed.as_ref().err().map(ToString::to_string),"release_error":released.as_ref().err().map(ToString::to_string)}),
        );
        match (observed, released) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error.to_string()),
            (Err(observer), Err(release)) => {
                Err(format!("observer: {observer}; release: {release}"))
            }
        }
    }

    // Every scenario ends admission before unblocking workers or closing native helpers.
    // Preserve the scenario failure even when native termination also fails.
    fn finish<T>(
        session: &mut Session,
        result: Result<T, Box<dyn std::error::Error>>,
        success_reason: OperationTerminalReason,
    ) -> Result<T, Box<dyn std::error::Error>> {
        session.fence(if result.is_ok() {
            success_reason
        } else {
            OperationTerminalReason::AuthorityUncertain
        });
        session.platform.interrupt_validation_read();
        let scenario_error = result.as_ref().err().map(ToString::to_string);
        let cleanup = close(session, scenario_error.as_deref());
        match (result, cleanup) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), Ok(())) => Err(error),
            (Ok(_), Err(cleanup)) => Err(format!("cleanup failed: {cleanup}").into()),
            (Err(error), Err(cleanup)) => Err(format!("{error}; cleanup failed: {cleanup}").into()),
        }
    }

    async fn prepare(
        binaries: &Path,
        fixture: Fixture,
        evidence: &Path,
    ) -> Result<(Session, Candidate), Box<dyn std::error::Error>> {
        let mut session = select(FixturePlatform::new(binaries, fixture)?, fixture).await?;
        let result = async {
            let read = session.start_read(1)?;
            let built = acquire(
                session.platform.clone(),
                &session.target_set,
                session.fixture,
                read,
                evidence,
            )
            .await?;
            Ok(built)
        }
        .await;
        match result {
            Ok(built) => Ok((session, built)),
            Err(error) => finish(
                &mut session,
                Err(error),
                OperationTerminalReason::AuthorityUncertain,
            ),
        }
    }

    async fn continuous(
        binaries: &Path,
        evidence: &Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut a = select(
            FixturePlatform::new(binaries, Fixture::A)?.with_continuous_reads(),
            Fixture::A,
        )
        .await?;
        let result = async {
            let a1 = a.start_read(1)?;
            let first = acquire(a.platform.clone(), &a.target_set, a.fixture, a1, evidence).await?;
            accept(&mut a, &first)?;
            export(&first, evidence, "a-read-1")?;

            let a2 = a.start_read(2)?;
            check(a2.target == a1.target, "refresh replaced A's receipt")?;
            let second =
                acquire(a.platform.clone(), &a.target_set, a.fixture, a2, evidence).await?;
            check(
                accept(&mut a, &first).is_err()
                    && a.admission.admits(a2)
                    && a.platform.approve_read_publication(a1).is_err(),
                "old A read replaced or consumed the new read",
            )?;
            log("rejected-superseded-read", json!({"old":a1,"current":a2}));
            accept(&mut a, &second)?;
            export(&second, evidence, "a-read-2")?;

            let a3 = a.start_read(3)?;
            let held = acquire(a.platform.clone(), &a.target_set, a.fixture, a3, evidence).await?;
            a.platform.destroy_owned_fixture(a3.target)?;
            let notification = tokio::time::timeout(
                Duration::from_secs(2),
                poll_fn(|cx| a.observation.events.poll_next(cx)),
            )
            .await?;
            check(
                notification.as_ref().is_some_and(|event| {
                    event.target == a.request.target
                        && event.context_id == a.request.context_id
                        && event.source_registration_id == a.request.source_registration_id
                        && event.observer_epoch == a.request.observer_epoch
                        && event.notification == WindowObservationNotification::WindowDestroyed
                }),
                "native destruction was not correlated to the retained A observer",
            )?;
            log(
                "native-invalidation",
                json!({"read":a3,"notification":notification}),
            );
            let denied = a
                .platform
                .approve_read_publication(a3)
                .err()
                .ok_or("native authority accepted the held A result")?;
            check(
                denied.to_string().contains("authority lost"),
                "unexpected native rejection",
            )?;
            a.fence(OperationTerminalReason::AuthorityUncertain);
            check(
                accept(&mut a, &held).is_err(),
                "held A result was admitted after destruction",
            )?;
            check(a.start_read(4).is_err(), "destroyed A issued another read")?;
            log(
                "rejected-invalidated-read",
                json!({"read":a3,"native":denied.to_string(),"new_read_denied":true}),
            );
            Ok((a1, a2, a3, held))
        }
        .await;
        let (a1, a2, a3, held) =
            finish(&mut a, result, OperationTerminalReason::AuthorityUncertain)?;

        let second_read = ReadSequence::FIRST.checked_next().expect("second read");
        let mut b = select(
            FixturePlatform::new(binaries, Fixture::B)?
                .with_continuous_reads()
                .with_read_probe_at(second_read, FixtureReadProbe::Late),
            Fixture::B,
        )
        .await?;
        let result = async {
        let b1 = b.start_read(1)?;
        check(
            b1.target.operation_id != a3.target.operation_id
                && b1.target.receipt != a3.target.receipt,
            "reselection reused A's operation or receipt",
        )?;
        check(
            accept(&mut b, &held).is_err() && b.admission.admits(b1),
            "reselected B admitted old A or consumed B's read",
        )?;
        log("rejected-old-generation", json!({"old":a3,"current":b1}));
        let selected = acquire(b.platform.clone(), &b.target_set, b.fixture, b1, evidence).await?;
        accept(&mut b, &selected)?;
        export(&selected, evidence, "b-read-1")?;

        let b2 = b.start_read(2)?;
        let targets = b.target_set.clone();
        let mut acquiring = Box::pin(acquire(b.platform.clone(), &targets, b.fixture, b2, evidence));
        let waiting = async {
            while !b.platform.validation_waiting() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        tokio::select! {
            early = &mut acquiring => {
                early?;
                return Err("second B read completed before the Stop probe".into());
            }
            seen = tokio::time::timeout(Duration::from_secs(8), waiting) => seen?,
        }
        check(
            b.admission.admits(b2),
            "Stop probe is not the current B read",
        )?;
        log("stop-requested", json!({"read":b2,"worker_waiting":true}));
        b.fence(OperationTerminalReason::Cancelled);
        b.platform.deliver_validation_result();
        let late = acquiring.await?;
        check(
            accept(&mut b, &late).is_err(),
            "Stop admitted the late B result",
        )?;
        check(b.start_read(3).is_err(), "stopped B issued another read")?;
        log(
            "rejected-stopped-read",
            json!({"read":late.read,"digest":late.projection.digest().as_str(),"new_read_denied":true}),
        );
        Ok((b1,b2))
        }.await;
        let (b1, b2) = finish(&mut b, result, OperationTerminalReason::Cancelled)?;
        log(
            "continuous-complete",
            json!({"published":[a1,a2,b1],"rejected_after_terminal":[a3,b2]}),
        );
        Ok(())
    }

    pub(super) async fn run() -> Result<(), Box<dyn std::error::Error>> {
        let [binaries, evidence, mode]: [_; 3] = env::args_os().skip(1).collect::<Vec<_>>().try_into()
            .map_err(|_| "usage: cooperative_validation <fixture binaries> <fresh evidence dir> <matrix|cancel|continuous>")?;
        let evidence = Path::new(&evidence);
        let binaries = Path::new(&binaries);
        fs::create_dir(evidence)?;
        if mode == "cancel" {
            let platform = FixturePlatform::new(binaries, Fixture::A)?;
            let operation = Uuid::new_v4();
            let mut registered = false;
            let result: Result<(), Box<dyn std::error::Error>> = async {
                let reply = platform.pick(operation).await?;
                registered = matches!(&reply, WindowPickerReply::Selected { .. });
                check(
                    matches!(reply, WindowPickerReply::Cancelled),
                    "select Cancel in the fixture dialog",
                )?;
                check(
                    !platform.inspect(),
                    "cancelled selection registered a target",
                )?;
                log("selection-cancelled", json!({"operation":operation}));
                Ok(())
            }
            .await;
            let mut admission = ReadAdmission::new(operation);
            admission.terminate(if result.is_ok() {
                OperationTerminalReason::Cancelled
            } else {
                OperationTerminalReason::AuthorityUncertain
            });
            admission.begin_cleanup();
            let released = platform.release_operation(operation);
            admission.finish_cleanup(if registered && released.is_ok() {
                CleanupOutcome::ConfirmedClosed
            } else {
                CleanupOutcome::UnconfirmedTermination
            });
            log(
                "cleanup",
                json!({"operation":operation,"phase":"cancel","outcome":format!("{:?}",admission.cleanup_state()),"scenario_error":result.as_ref().err().map(ToString::to_string),"registered_release":registered,"unregistered_teardown_unobserved":!registered,"release_error":released.as_ref().err().map(ToString::to_string)}),
            );
            return match (result, released) {
                (Ok(()), Ok(())) => Ok(()),
                (Err(error), Ok(())) => Err(error),
                (Ok(()), Err(cleanup)) => Err(format!("cleanup failed: {cleanup}").into()),
                (Err(error), Err(cleanup)) => {
                    Err(format!("{error}; cleanup failed: {cleanup}").into())
                }
            };
        }
        if mode == "continuous" {
            return continuous(binaries, evidence).await;
        }
        check(mode == "matrix", "unknown validation mode")?;

        let (mut initial, first) = prepare(binaries, Fixture::A, evidence).await?;
        let result = async {
            accept(&mut initial, &first)?;
            export(&first, evidence, "initial-a")?;
            check(
                accept(&mut initial, &first).is_err(),
                "duplicate initial publication accepted",
            )?;
            log("rejected-duplicate", json!({"read":first.read}));
            Ok(())
        }
        .await;
        finish(&mut initial, result, OperationTerminalReason::Completed)?;

        let (mut lost, held) = prepare(binaries, Fixture::A, evidence).await?;
        let old = held.read;
        let result = async {
            lost.platform.terminate_owned_fixture(old.target)?;
            let notification = tokio::time::timeout(
                Duration::from_secs(2),
                poll_fn(|cx| lost.observation.events.poll_next(cx)),
            )
            .await?;
            log(
                "native-invalidation",
                json!({"read":old,"notification":notification}),
            );
            let denied = accept(&mut lost, &held)
                .err()
                .ok_or("held result published after owner loss")?;
            check(
                denied.contains("authority lost"),
                "held result was rejected without the native gate",
            )?;
            log("rejected-owner-loss", json!({"read":old,"reason":denied}));
            Ok(())
        }
        .await;
        finish(
            &mut lost,
            result,
            OperationTerminalReason::AuthorityUncertain,
        )?;
        check(!lost.admission.admits(old), "old read remained admitted")?;

        let (mut selected, latest) = prepare(binaries, Fixture::B, evidence).await?;
        let result = async {
            let new = latest.read;
            check(
                new.target != old.target,
                "reselection reused the old target receipt",
            )?;
            check(
                accept(&mut selected, &held).is_err(),
                "old-generation result accepted into selected B",
            )?;
            check(
                selected.admission.admits(new),
                "old result consumed B's read",
            )?;
            log("rejected-old-generation", json!({"old":old,"current":new}));
            accept(&mut selected, &latest)?;
            export(&latest, evidence, "reselected-b")?;
            Ok(())
        }
        .await;
        finish(&mut selected, result, OperationTerminalReason::Completed)?;
        Ok(())
    }
}
