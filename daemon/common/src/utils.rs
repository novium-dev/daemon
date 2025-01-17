use lazy_static::lazy_static;
use regex::Regex;

lazy_static! {
    static ref CGROUP_REGEX: Regex = Regex::new("docker-([a-z0-9]{64}).scope").unwrap();
}

#[cfg(feature = "docker")]
pub async fn identify_container_by_pid(
    docker: &bollard::Docker,
    pid: i32,
) -> Result<u32, std::io::Error> {
    use std::io;
    use std::io::ErrorKind;
    use tokio::fs::File;
    use tokio::io::AsyncReadExt;

    let mut file = File::open(format!("/proc/{}/cgroup", pid)).await?;
    let mut buf = String::new();
    file.read_to_string(&mut buf).await?;
    let captures = CGROUP_REGEX.captures(&buf);
    match captures {
        None => Err(io::Error::new(ErrorKind::Other, "Not in a docker cgroup")),
        Some(captures) => {
            let capture = &captures[1];
            let container = match docker.inspect_container(capture, None).await {
                Ok(container) => container,
                Err(err) => return Err(io::Error::new(ErrorKind::Other, err)),
            };
            match container.name {
                None => Err(io::Error::new(
                    ErrorKind::Other,
                    "Container unknown (name missing)",
                )),
                Some(name) => {
                    let mut name: &str = &name;
                    if name.starts_with("/") {
                        name = &name[1..];
                    }
                    if !name.starts_with(crate::CONTAINER_PREFIX) {
                        return Err(io::Error::new(
                            ErrorKind::Other,
                            format!("Container unknown (not starting with prefix: {:?})", name),
                        ));
                    }
                    let str = &name[crate::CONTAINER_PREFIX.len()..];
                    str.parse()
                        .map_err(|err| io::Error::new(ErrorKind::Other, err))
                }
            }
        }
    }
}
