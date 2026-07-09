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
    { package: "example-config-library", latest_version: "0.8.3", last_published: "2026-06-21T10:14:00Z" },
    { package: "example-deploy-tools", latest_version: "1.4.1", last_published: "2026-06-18T08:40:00Z" },
    { package: "example-aws-constructs", latest_version: "2.2.0", last_published: "2026-06-10T13:25:00Z" },
    { package: "example-policy-pack", latest_version: "0.5.7", last_published: "2026-05-29T15:02:00Z" },
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
