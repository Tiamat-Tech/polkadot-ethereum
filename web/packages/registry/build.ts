import { assetsV2, environment } from "@snowbridge/api"
import { writeFile } from "fs/promises"

async function buildRegistry(env: string, options: assetsV2.RegistryOptions) {
    const registry = await assetsV2.buildRegistry(options)
    const json = JSON.stringify(
        registry,
        (key, value) => {
            if (typeof value === "bigint") {
                return `bigint:${value.toString()}`
            }
            return value
        },
        2
    )

    const filepath = `src/${env}.registry.json`
    await writeFile(filepath, json)
}

(async () => {
    const envs = [
        "polkadot_mainnet",
        "westend_sepolia",
        "paseo_sepolia",
    ]
    const apiKey = process.env.ETHEREUM_API_KEY
    if(!apiKey || apiKey.trim().length === 0) {
        throw Error(`ETHEREUM_API_KEY env variable not set.`)
    }
    await Promise.all(envs.map(async env => {
        const options = assetsV2.fromEnvironment(environment.SNOWBRIDGE_ENV[env])
        await buildRegistry(env, options)
    }))
})()
