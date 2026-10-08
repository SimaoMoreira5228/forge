#[derive(Debug, Clone)]
pub struct JunitCase {
	pub component: String,
	pub verdict: String,
	pub duration_ms: i64,
	pub stderr: String,
}

pub fn junit_xml(results: &[JunitCase]) -> String {
	let mut packages: Vec<&str> = results.iter().map(|r| package_of(&r.component)).collect();
	packages.sort_unstable();
	packages.dedup();

	let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites>\n");
	for package in &packages {
		let suite: Vec<&JunitCase> = results.iter().filter(|r| package_of(&r.component) == *package).collect();
		let tests = suite.len();
		let failures = suite.iter().filter(|r| r.verdict == "FAILED").count();
		let time: f64 = suite.iter().map(|r| r.duration_ms as f64 / 1000.0).sum();

		out.push_str(&format!(
			"  <testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" skipped=\"0\" time=\"{time:.3}\">\n",
			escape(package),
			tests,
			failures,
		));
		for result in &suite {
			let duration = result.duration_ms as f64 / 1000.0;
			if result.verdict == "PASSED" {
				out.push_str(&format!(
					"    <testcase classname=\"{}\" name=\"{}\" time=\"{duration:.3}\"/>\n",
					escape(package),
					escape(short_name(&result.component)),
				));
			} else {
				out.push_str(&format!(
					"    <testcase classname=\"{}\" name=\"{}\" time=\"{duration:.3}\">\n",
					escape(package),
					escape(short_name(&result.component)),
				));
				out.push_str(&format!(
					"      <failure message=\"exit code nonzero\">{}</failure>\n",
					escape(&result.stderr),
				));
				out.push_str("    </testcase>\n");
			}
		}
		out.push_str("  </testsuite>\n");
	}
	out.push_str("</testsuites>\n");
	out
}

fn package_of(component_label: &str) -> &str {
	match component_label.rsplit_once(':') {
		Some((package, _)) => package,
		None => component_label,
	}
}

fn short_name(component_label: &str) -> &str {
	component_label.rsplit(':').next().unwrap_or(component_label)
}

fn escape(text: &str) -> String {
	text.replace('&', "&amp;")
		.replace('<', "&lt;")
		.replace('>', "&gt;")
		.replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
	use super::*;

	fn case(component: &str, verdict: &str, ms: i64) -> JunitCase {
		JunitCase {
			component: component.into(),
			verdict: verdict.into(),
			duration_ms: ms,
			stderr: String::new(),
		}
	}

	#[test]
	fn groups_by_package_and_counts_failures() {
		let xml = junit_xml(&[
			case("//a:t1", "PASSED", 10),
			case("//b:t2", "FAILED", 20),
			case("//a:t3", "PASSED", 5),
		]);
		assert!(xml.contains("<testsuite name=\"//a\" tests=\"2\" failures=\"0\""), "{xml}");
		assert!(xml.contains("<testsuite name=\"//b\" tests=\"1\" failures=\"1\""), "{xml}");
		assert!(xml.matches("<failure").count() == 1);
		assert!(xml.ends_with("</testsuites>\n"));
	}

	#[test]
	fn escapes_control_characters_in_failure_output() {
		let mut failed = case("//x:t", "FAILED", 1);
		failed.stderr = "<weird> & \"quoted\"".into();
		let xml = junit_xml(&[failed]);
		assert!(xml.contains("&lt;weird&gt; &amp; &quot;quoted&quot;"), "{xml}");
	}
}
