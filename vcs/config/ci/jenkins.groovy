// =============================================================================
// Verum Compliance Suite - Jenkins Pipeline
// =============================================================================
//
// Production-grade CI/CD configuration for Jenkins
//
// Pipeline Stages:
//   1. Build         - Build VCS tools and compiler
//   2. L0 Critical   - Critical tests (100% required)
//   3. L1 Core       - Core tests (100% required)
//   4. L2 Standard   - Standard tests (95%+ required)
//   5. Differential  - Cross-tier semantic equivalence
//   6. Extended      - L3-L4 tests (nightly)
//   7. Analysis      - Fuzzing, benchmarks, coverage
//   8. Report        - Generate reports and notifications
//
// Usage:
//   Configure Jenkins to use this Jenkinsfile from the repository
//   or include it via pipeline libraries.
//
// Requirements:
//   - Docker agent with Rust toolchain
//   - Credentials: SLACK_WEBHOOK, CODECOV_TOKEN (optional)
//
// Reference: VCS Spec Section 23 - CI/CD Integration
// =============================================================================

@Library('shared-library') _

pipeline {
    agent {
        docker {
            image 'rust:latest'
            args '-v /var/run/docker.sock:/var/run/docker.sock'
        }
    }

    options {
        timeout(time: 4, unit: 'HOURS')
        timestamps()
        buildDiscarder(logRotator(numToKeepStr: '30', artifactNumToKeepStr: '10'))
        disableConcurrentBuilds()
        ansiColor('xterm')
    }

    environment {
        CARGO_HOME = "${WORKSPACE}/.cargo"
        CARGO_TERM_COLOR = 'always'
        CARGO_INCREMENTAL = '0'
        RUST_BACKTRACE = '1'
        VCS_PARALLEL = '8'
        VCS_TIMEOUT_DEFAULT = '30000'
        VCS_TIMEOUT_EXTENDED = '600000'
        VCS_CONFIG = 'config/vcs.toml'
    }

    parameters {
        choice(
            name: 'TEST_LEVEL',
            choices: ['L0,L1', 'L0,L1,L2', 'all'],
            description: 'Test levels to run'
        )
        string(
            name: 'FUZZ_DURATION',
            defaultValue: '30m',
            description: 'Fuzzing duration (e.g., 10m, 30m, 1h)'
        )
        booleanParam(
            name: 'RUN_BENCHMARKS',
            defaultValue: false,
            description: 'Run performance benchmarks'
        )
        booleanParam(
            name: 'RUN_FUZZING',
            defaultValue: false,
            description: 'Run fuzz testing'
        )
        booleanParam(
            name: 'RUN_COVERAGE',
            defaultValue: false,
            description: 'Generate code coverage report'
        )
    }

    stages {
        // =====================================================================
        // Build Stage
        // =====================================================================
        stage('Build') {
            parallel {
                stage('Build vtest') {
                    steps {
                        dir('vcs/runner/vtest') {
                            sh 'cargo build --release'
                        }
                    }
                }
                stage('Build vfuzz') {
                    steps {
                        dir('vcs/runner/vfuzz') {
                            sh 'cargo build --release --features parallel'
                        }
                    }
                }
                stage('Build vbench') {
                    steps {
                        dir('vcs/runner/vbench') {
                            sh 'cargo build --release'
                        }
                    }
                }
            }
        }

        // =====================================================================
        // L0 Critical Tests
        // =====================================================================
        stage('L0 Critical') {
            steps {
                dir('vcs') {
                    sh '''
                        mkdir -p reports
                        ../runner/vtest/target/release/vtest run \
                            --level L0 \
                            --parallel ${VCS_PARALLEL} \
                            --timeout ${VCS_TIMEOUT_DEFAULT} \
                            --format junit \
                            --output reports/l0-results.xml \
                            --config ${VCS_CONFIG} \
                            --threshold 100.0
                    '''
                }
            }
            post {
                always {
                    junit 'vcs/reports/l0-results.xml'
                }
                failure {
                    script {
                        currentBuild.result = 'FAILURE'
                        error('L0 Critical tests failed - pipeline blocked')
                    }
                }
            }
        }

        // =====================================================================
        // L1 Core Tests
        // =====================================================================
        stage('L1 Core') {
            steps {
                dir('vcs') {
                    sh '''
                        ../runner/vtest/target/release/vtest run \
                            --level L1 \
                            --parallel ${VCS_PARALLEL} \
                            --timeout ${VCS_TIMEOUT_DEFAULT} \
                            --format junit \
                            --output reports/l1-results.xml \
                            --config ${VCS_CONFIG} \
                            --threshold 100.0
                    '''
                }
            }
            post {
                always {
                    junit 'vcs/reports/l1-results.xml'
                }
                failure {
                    script {
                        currentBuild.result = 'FAILURE'
                        error('L1 Core tests failed - pipeline blocked')
                    }
                }
            }
        }

        // =====================================================================
        // L2 Standard Tests
        // =====================================================================
        stage('L2 Standard') {
            steps {
                dir('vcs') {
                    sh '''
                        ../runner/vtest/target/release/vtest run \
                            --level L2 \
                            --parallel ${VCS_PARALLEL} \
                            --timeout ${VCS_TIMEOUT_DEFAULT} \
                            --format json \
                            --output reports/l2-results.json \
                            --config ${VCS_CONFIG} \
                            --threshold 95.0
                    '''

                    script {
                        def results = readJSON file: 'reports/l2-results.json'
                        def passRate = results.summary.pass_percentage ?: 0
                        echo "L2 Pass Rate: ${passRate}%"

                        if (passRate < 95.0) {
                            error("L2 tests below 95% threshold (${passRate}%)")
                        }
                    }
                }
            }
            post {
                failure {
                    script {
                        currentBuild.result = 'FAILURE'
                    }
                }
            }
        }

        // =====================================================================
        // Differential Tests
        // =====================================================================
        stage('Differential') {
            parallel {
                stage('Tier 0 vs Tier 3') {
                    steps {
                        dir('vcs') {
                            sh '''
                                ../runner/vtest/target/release/vtest run \
                                    differential/ \
                                    --tier 0,3 \
                                    --parallel ${VCS_PARALLEL} \
                                    --format junit \
                                    --output reports/differential-results.xml \
                                    --config ${VCS_CONFIG}

                                if ! grep -q 'failures="0"' reports/differential-results.xml 2>/dev/null; then
                                    echo "ERROR: Differential tests failed"
                                    exit 1
                                fi
                            '''
                        }
                    }
                    post {
                        always {
                            junit 'vcs/reports/differential-results.xml'
                        }
                    }
                }
            }
        }

        // =====================================================================
        // Extended Tests (L3-L4) - Nightly or Manual
        // =====================================================================
        stage('Extended Tests') {
            when {
                anyOf {
                    triggeredBy 'TimerTrigger'
                    expression { params.TEST_LEVEL == 'all' }
                }
            }
            parallel {
                stage('L3 Extended') {
                    steps {
                        dir('vcs') {
                            sh '''
                                ../runner/vtest/target/release/vtest run \
                                    --level L3 \
                                    --parallel ${VCS_PARALLEL} \
                                    --timeout ${VCS_TIMEOUT_EXTENDED} \
                                    --format json \
                                    --output reports/l3-results.json \
                                    --config ${VCS_CONFIG} \
                                    --threshold 90.0 || true
                            '''

                            script {
                                if (fileExists('reports/l3-results.json')) {
                                    def results = readJSON file: 'reports/l3-results.json'
                                    def passRate = results.summary?.pass_percentage ?: 0
                                    echo "L3 Pass Rate: ${passRate}%"

                                    if (passRate < 90.0) {
                                        unstable("L3 tests below 90% threshold (${passRate}%)")
                                    }
                                }
                            }
                        }
                    }
                }
                stage('L4 Performance') {
                    steps {
                        dir('vcs') {
                            sh '''
                                ../runner/vtest/target/release/vtest run \
                                    --level L4 \
                                    --parallel 4 \
                                    --timeout ${VCS_TIMEOUT_EXTENDED} \
                                    --format json \
                                    --output reports/l4-results.json \
                                    --config ${VCS_CONFIG} || true
                            '''

                            script {
                                if (fileExists('reports/l4-results.json')) {
                                    def results = readJSON file: 'reports/l4-results.json'
                                    echo "L4 Performance Results (advisory):"
                                    echo results.summary?.toString() ?: 'No summary available'
                                }
                            }
                        }
                    }
                }
            }
        }

        // =====================================================================
        // Analysis Stage
        // =====================================================================
        stage('Analysis') {
            parallel {
                // Fuzzing
                stage('Fuzzing') {
                    when {
                        anyOf {
                            triggeredBy 'TimerTrigger'
                            expression { params.RUN_FUZZING }
                        }
                    }
                    steps {
                        dir('vcs') {
                            sh """
                                mkdir -p fuzz/crashes fuzz/artifacts

                                ../runner/vfuzz/target/release/vfuzz run \
                                    --targets all \
                                    --duration ${params.FUZZ_DURATION} \
                                    --parallel 4 \
                                    --corpus fuzz/seeds/ \
                                    --crashes fuzz/crashes/ \
                                    --config ${VCS_CONFIG} || true

                                CRASH_COUNT=\$(find fuzz/crashes -type f 2>/dev/null | wc -l)
                                if [ "\$CRASH_COUNT" -gt 0 ]; then
                                    echo "WARNING: Fuzzing found \$CRASH_COUNT crashes"
                                    ls -la fuzz/crashes/
                                else
                                    echo "No crashes found during fuzzing"
                                fi
                            """
                        }
                    }
                    post {
                        always {
                            archiveArtifacts artifacts: 'vcs/fuzz/crashes/**/*',
                                allowEmptyArchive: true,
                                fingerprint: true
                        }
                    }
                }

                // Benchmarks
                stage('Benchmarks') {
                    when {
                        anyOf {
                            triggeredBy 'TimerTrigger'
                            expression { params.RUN_BENCHMARKS }
                        }
                    }
                    steps {
                        dir('vcs') {
                            sh '''
                                mkdir -p reports baselines

                                # Run benchmarks
                                ../runner/vbench/target/release/vbench run \
                                    --suite all \
                                    --iterations 1000 \
                                    --warmup 100 \
                                    --config ${VCS_CONFIG} \
                                    --format json \
                                    --output reports/benchmark-results.json

                                # Compare to baseline if exists
                                if [ -f baselines/benchmark-baseline.json ]; then
                                    ../runner/vbench/target/release/vbench compare \
                                        --current reports/benchmark-results.json \
                                        --baseline baselines/benchmark-baseline.json \
                                        --threshold 10.0 \
                                        --format markdown \
                                        --output reports/benchmark-comparison.md || true
                                fi

                                # Validate thresholds
                                ../runner/vbench/target/release/vbench validate \
                                    --thresholds config/thresholds.toml \
                                    --results reports/benchmark-results.json || true
                            '''
                        }
                    }
                    post {
                        always {
                            archiveArtifacts artifacts: 'vcs/reports/benchmark-*.json,vcs/reports/benchmark-*.md',
                                allowEmptyArchive: true,
                                fingerprint: true
                        }
                    }
                }

                // Coverage
                stage('Coverage') {
                    when {
                        anyOf {
                            branch 'main'
                            expression { params.RUN_COVERAGE }
                        }
                    }
                    agent {
                        docker {
                            image 'xd009642/tarpaulin:develop-nightly'
                        }
                    }
                    steps {
                        sh '''
                            cargo tarpaulin --all-features --workspace \
                                --out Xml --out Html \
                                --output-dir coverage/ \
                                --skip-clean \
                                --timeout 600 || true
                        '''
                    }
                    post {
                        always {
                            publishCoverage adapters: [coberturaAdapter('coverage/cobertura.xml')],
                                sourceFileResolver: sourceFiles('NEVER_STORE')

                            archiveArtifacts artifacts: 'coverage/**/*',
                                allowEmptyArchive: true
                        }
                    }
                }
            }
        }

        // =====================================================================
        // Report Stage
        // =====================================================================
        stage('Report') {
            steps {
                dir('vcs') {
                    sh '''
                        chmod +x scripts/generate-report.sh
                        ./scripts/generate-report.sh \
                            --input reports/ \
                            --output reports/vcs-report.html \
                            --title "VCS CI Report - ${BUILD_NUMBER}" \
                            --format html
                    '''
                }
            }
            post {
                always {
                    publishHTML(target: [
                        allowMissing: true,
                        alwaysLinkToLastBuild: true,
                        keepAll: true,
                        reportDir: 'vcs/reports',
                        reportFiles: 'vcs-report.html',
                        reportName: 'VCS Test Report',
                        reportTitles: 'VCS Report'
                    ])

                    archiveArtifacts artifacts: 'vcs/reports/**/*',
                        allowEmptyArchive: true,
                        fingerprint: true
                }
            }
        }
    }

    post {
        always {
            // Clean workspace
            cleanWs(cleanWhenNotBuilt: false,
                deleteDirs: true,
                disableDeferredWipeout: true,
                notFailBuild: true,
                patterns: [
                    [pattern: '.cargo/**', type: 'EXCLUDE'],
                    [pattern: '**/target/**', type: 'INCLUDE']
                ])
        }

        success {
            script {
                if (env.SLACK_WEBHOOK) {
                    slackSend(
                        color: 'good',
                        message: "VCS Pipeline PASSED - ${env.JOB_NAME} #${env.BUILD_NUMBER}\n${env.BUILD_URL}",
                        channel: '#verum-ci'
                    )
                }
            }
        }

        failure {
            script {
                if (env.SLACK_WEBHOOK) {
                    slackSend(
                        color: 'danger',
                        message: "VCS Pipeline FAILED - ${env.JOB_NAME} #${env.BUILD_NUMBER}\n${env.BUILD_URL}",
                        channel: '#verum-ci'
                    )
                }

                // Send email notification
                emailext(
                    subject: "VCS Pipeline FAILED - ${env.JOB_NAME} #${env.BUILD_NUMBER}",
                    body: """
                        <h2>VCS Pipeline Failed</h2>
                        <p>Job: ${env.JOB_NAME}</p>
                        <p>Build: #${env.BUILD_NUMBER}</p>
                        <p>URL: <a href="${env.BUILD_URL}">${env.BUILD_URL}</a></p>
                    """,
                    recipientProviders: [developers(), requestor()],
                    mimeType: 'text/html'
                )
            }
        }

        unstable {
            script {
                if (env.SLACK_WEBHOOK) {
                    slackSend(
                        color: 'warning',
                        message: "VCS Pipeline UNSTABLE - ${env.JOB_NAME} #${env.BUILD_NUMBER}\n${env.BUILD_URL}",
                        channel: '#verum-ci'
                    )
                }
            }
        }
    }
}

// =============================================================================
// Helper Functions
// =============================================================================

def getPassRate(String jsonFile) {
    if (!fileExists(jsonFile)) {
        return 0.0
    }

    def results = readJSON file: jsonFile
    return results.summary?.pass_percentage ?: 0.0
}

def checkThreshold(double actual, double threshold, String testName) {
    if (actual < threshold) {
        error("${testName} below ${threshold}% threshold (${actual}%)")
    }
    echo "${testName}: ${actual}% pass rate (threshold: ${threshold}%)"
}
