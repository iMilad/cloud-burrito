// Faked AWS responses for the mockup. Shaped roughly like what the
// real adapter layer (boto3 / steampipe / saw) would return.

const Mock = (() => {
  const accounts = [
    { id: "acct-prod", alias: "prod" },
    { id: "acct-staging", alias: "staging" },
    { id: "acct-dev", alias: "dev" },
    { id: "acct-shared", alias: "shared-services" },
    { id: "acct-data", alias: "data-lake" },
  ];

  const regions = ["eu-central-1", "eu-west-1", "us-east-1", "us-west-2"];

  const pipelineRuns = [
    { id: "exec-9f2a8c1b-0042", status: "Failed",     time: "2m ago",  trigger: "main@a1f3e9c", actor: "alice" },
    { id: "exec-9f2a8c1b-0041", status: "Succeeded",  time: "1h ago",  trigger: "main@7c2d8b1", actor: "alice" },
    { id: "exec-9f2a8c1b-0040", status: "Succeeded",  time: "3h ago",  trigger: "main@b9e4c0f", actor: "bob" },
    { id: "exec-9f2a8c1b-0039", status: "InProgress", time: "now",     trigger: "main@d3a7f12", actor: "alice" },
    { id: "exec-9f2a8c1b-0038", status: "Failed",     time: "yesterday", trigger: "main@5e1b6a8", actor: "carol" },
    { id: "exec-9f2a8c1b-0037", status: "Succeeded",  time: "yesterday", trigger: "main@88aa129", actor: "carol" },
    { id: "exec-9f2a8c1b-0036", status: "Succeeded",  time: "2 days",  trigger: "main@2b6a8c0", actor: "alice" },
    { id: "exec-9f2a8c1b-0035", status: "Stopped",    time: "2 days",  trigger: "manual",      actor: "bob" },
    { id: "exec-9f2a8c1b-0034", status: "Succeeded",  time: "3 days",  trigger: "main@e0c1d2f", actor: "alice" },
    { id: "exec-9f2a8c1b-0033", status: "Succeeded",  time: "4 days",  trigger: "main@1a9b3c4", actor: "alice" },
  ];

  // Faked CFN stacks tree
  const stacks = [
    {
      name: "uc-payment-service-prod",
      status: "UPDATE_ROLLBACK_COMPLETE",
      resources: [
        { name: "PaymentLambdaV2",   type: "AWS::Lambda::Function" },
        { name: "PaymentLambdaRole", type: "AWS::IAM::Role" },
        { name: "PaymentDB",         type: "AWS::RDS::DBInstance" },
        { name: "PaymentApi",        type: "AWS::ApiGatewayV2::Api" },
      ],
    },
    {
      name: "uc-order-service-prod",
      status: "UPDATE_COMPLETE",
      resources: [
        { name: "OrderLambda",  type: "AWS::Lambda::Function" },
        { name: "OrderQueue",   type: "AWS::SQS::Queue" },
        { name: "OrderTable",   type: "AWS::DynamoDB::Table" },
      ],
    },
    {
      name: "uc-inventory-service-prod",
      status: "UPDATE_COMPLETE",
      resources: [
        { name: "InventoryLambda", type: "AWS::Lambda::Function" },
        { name: "InventoryTable",  type: "AWS::DynamoDB::Table" },
      ],
    },
    {
      name: "shared-vpc-prod",
      status: "CREATE_COMPLETE",
      resources: [
        { name: "VPC",        type: "AWS::EC2::VPC" },
        { name: "PrivateSubnetA", type: "AWS::EC2::Subnet" },
        { name: "PrivateSubnetB", type: "AWS::EC2::Subnet" },
      ],
    },
  ];

  // Reverse lookup index
  const allResources = stacks.flatMap(s =>
    s.resources.map(r => ({
      stack: s.name,
      stackStatus: s.status,
      name: r.name,
      type: r.type,
      arn: arnFor(r.type, r.name),
    }))
  );

  function arnFor(type, name) {
    const account = "acct-demo";
    const region = "eu-central-1";
    if (type.includes("Lambda")) return `arn:aws:lambda:${region}:${account}:function:${name}`;
    if (type.includes("DynamoDB")) return `arn:aws:dynamodb:${region}:${account}:table/${name}`;
    if (type.includes("SQS")) return `arn:aws:sqs:${region}:${account}:${name}`;
    if (type.includes("RDS")) return `arn:aws:rds:${region}:${account}:db:${name}`;
    if (type.includes("ApiGateway")) return `arn:aws:apigateway:${region}::/apis/${name}`;
    if (type.includes("EC2::VPC")) return `arn:aws:ec2:${region}:${account}:vpc/${name}`;
    if (type.includes("Subnet")) return `arn:aws:ec2:${region}:${account}:subnet/${name}`;
    if (type.includes("IAM")) return `arn:aws:iam::${account}:role/${name}`;
    return `arn:aws:?:${region}:${account}:${name}`;
  }

  // Errors-by-stack aggregation (bars)
  const errorsByStack = [
    { stack: "uc-payment-service-prod",   errors: 247 },
    { stack: "uc-order-service-prod",     errors: 89 },
    { stack: "uc-inventory-service-prod", errors: 31 },
    { stack: "shared-vpc-prod",           errors: 6 },
    { stack: "uc-shipping-service-prod",  errors: 2 },
  ];

  const codeArtifactPackages = [
    {
      package: "example-config-library",
      latest_version: "1.2.1.260701.093637+9c0581c",
      last_published: "2026-07-01T09:36:37Z",
      versions: [
        { version: "1.2.1.260701.093637+9c0581c", published: "2026-07-01T09:36:37Z" },
        { version: "1.2.0.260624.133554+7e8c4d2", published: "2026-06-24T13:35:54Z" },
        { version: "1.1.1.260624.064731+4f91bc8", published: "2026-06-24T06:47:31Z" },
        { version: "1.1.1.260520.083120+7d70ced", published: "2026-05-20T08:31:20Z" },
        { version: "1.1.0.260520.082952+a6e6f44", published: "2026-05-20T08:29:52Z" },
        { version: "1.1.0.260218.160335+8bd32a1", published: "2026-02-18T16:03:35Z" },
        { version: "1.1.0.260218.123958+6ac417e", published: "2026-02-18T12:39:58Z" },
        { version: "1.1.0.260218.123329+d910be7", published: "2026-02-18T12:33:29Z" },
        { version: "1.1.0.260218.115705+e04c4a9", published: "2026-02-18T11:57:05Z" },
        { version: "1.1.0.260217.143437+14d2544", published: "2026-02-17T14:34:37Z" },
      ],
    },
    {
      package: "example-deploy-tools",
      latest_version: "1.4.1",
      last_published: "2026-06-18T08:40:00Z",
      versions: [
        { version: "1.4.1", published: "2026-06-18T08:40:00Z" },
        { version: "1.4.0", published: "2026-06-04T11:12:00Z" },
        { version: "1.3.2", published: "2026-05-22T14:06:00Z" },
        { version: "1.3.1", published: "2026-05-06T09:31:00Z" },
        { version: "1.3.0", published: "2026-04-15T12:45:00Z" },
        { version: "1.2.1", published: "2026-03-28T07:58:00Z" },
      ],
    },
    {
      package: "example-aws-constructs",
      latest_version: "2.2.0",
      last_published: "2026-06-10T13:25:00Z",
      versions: [
        { version: "2.2.0", published: "2026-06-10T13:25:00Z" },
        { version: "2.1.1", published: "2026-05-19T16:18:00Z" },
        { version: "2.1.0", published: "2026-04-30T10:42:00Z" },
        { version: "2.0.0", published: "2026-03-11T08:05:00Z" },
      ],
    },
    {
      package: "example-policy-pack",
      latest_version: "0.5.7",
      last_published: "2026-05-29T15:02:00Z",
      versions: [
        { version: "0.5.7", published: "2026-05-29T15:02:00Z" },
        { version: "0.5.6", published: "2026-05-13T09:27:00Z" },
        { version: "0.5.5", published: "2026-04-24T11:49:00Z" },
        { version: "0.5.4", published: "2026-04-02T13:36:00Z" },
        { version: "0.5.3", published: "2026-03-17T08:21:00Z" },
      ],
    },
  ];

  // A finite pool of fake log lines. The log-tail widget cycles through these.
  const logPool = [
    { level: "info",  msg: "START RequestId: 7a3e1c9b-0042 Version: 14" },
    { level: "info",  msg: "received event: { 'orderId': 'ord-8821', 'amount': 49.90, 'currency': 'EUR' }" },
    { level: "info",  msg: "validating payment payload" },
    { level: "info",  msg: "calling provider.charge(orderId=ord-8821)" },
    { level: "warn",  msg: "provider.charge took 1840ms (p95 budget 1200ms)" },
    { level: "info",  msg: "provider.charge ok, ref=pi_3PqL2K2eZ" },
    { level: "ok",    msg: "writing payment record to DynamoDB" },
    { level: "info",  msg: "publishing event PaymentCompleted to EventBridge" },
    { level: "info",  msg: "END RequestId: 7a3e1c9b-0042" },
    { level: "info",  msg: "REPORT Duration: 1923.41 ms  Billed: 2000 ms  Memory: 256 MB  Max Used: 142 MB" },
    { level: "error", msg: "ProviderTimeout after 3000ms invoking https://api.example.com/charge" },
    { level: "warn",  msg: "retrying request, attempt 2 of 3" },
    { level: "info",  msg: "request succeeded on retry" },
  ];

  return {
    accounts,
    regions,
    pipelineRuns,
    stacks,
    allResources,
    errorsByStack,
    codeArtifactPackages,
    logPool,
  };
})();
