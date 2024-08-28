use anyhow::Context;
use jsonptr::{Pointer, Token};
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    api::{Patch, PatchParams},
    runtime::wait,
    Api,
};

/// Returns new generation.
pub async fn patch_env_name(
    api: &Api<Deployment>,
    deployment: &Deployment,
    env_name: &str,
    new_value: &str,
) -> anyhow::Result<i64> {
    let name = deployment
        .metadata
        .name
        .as_ref()
        .context("deployment has no name")?;

    let patch_ops = deployment
        .spec
        .as_ref()
        .and_then(|spec| spec.template.spec.as_ref())
        .into_iter()
        .flat_map(|spec| spec.containers.iter().enumerate())
        .filter_map(|(container_idx, container)| {
            let env_idx = container
                .env
                .as_ref()?
                .iter()
                .position(|env| env.name == env_name)?;

            Some((container_idx, env_idx))
        })
        .map(|(container_idx, env_idx)| json_patch::ReplaceOperation {
            path: Pointer::new([
                Token::from("spec"),
                Token::from("template"),
                Token::from("spec"),
                Token::from("containers"),
                Token::from(container_idx),
                Token::from("env"),
                Token::from(env_idx),
                Token::from("value"),
            ]),
            value: serde_json::Value::String(new_value.to_string()),
        })
        .map(json_patch::PatchOperation::Replace)
        .collect::<Vec<_>>();

    anyhow::ensure!(!patch_ops.is_empty(), "no vars to patch");

    let deployment = api
        .patch(
            name,
            &PatchParams {
                field_manager: Some("kafka-splitting".into()),
                ..Default::default()
            },
            &Patch::<Deployment>::Json(json_patch::Patch(patch_ops)),
        )
        .await
        .context("failed to patch target resource")?;

    deployment
        .metadata
        .generation
        .context("failed to get new generation")
}

pub async fn wait_for_rollout_completion(
    api: Api<Deployment>,
    name: &str,
    new_generation: i64,
) -> anyhow::Result<()> {
    wait::await_condition(api, name, |deploy: Option<&Deployment>| {
        let Some(deploy) = deploy else {
            return true;
        };

        let Some(observed_generation) = deploy
            .status
            .as_ref()
            .and_then(|status| status.observed_generation)
        else {
            return false;
        };

        if observed_generation < new_generation {
            return false;
        }

        let Some(replicas) = deploy.status.as_ref().and_then(|status| status.replicas) else {
            return false;
        };
        let Some(updated_replicas) = deploy
            .status
            .as_ref()
            .and_then(|status| status.updated_replicas)
        else {
            return false;
        };

        updated_replicas == replicas
    })
    .await?
    .context("deployment was deleted")?;

    Ok(())
}
